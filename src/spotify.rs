// Spotify access via librespot: logging in, fetching metadata and downloading
// the audio stream.
//
// Login happens over Zeroconf: the ripper advertises itself as a Spotify
// Connect device and you pick it once from the official Spotify app; the
// resulting reusable credentials are stored on disk and reused on later runs.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use futures_util::StreamExt;
use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::DeviceType;
use librespot_core::{FileId, Session, SessionConfig, SpotifyId, SpotifyUri};
use librespot_discovery::Discovery;
use librespot_metadata::audio::{AudioFileFormat, AudioItem};
use librespot_metadata::{Album, Artist, Metadata, Playlist, Track};
use librespot_protocol::authentication::AuthenticationType;
use protobuf::{Message, UnknownValueRef};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::to_pretty_json;
use crate::outln;
use crate::util::{cache_dir, settings_dir};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// Image host (CDN) for album cover art referenced by the protocol metadata.
const IMAGE_URL: &str = "https://i.scdn.co/image/";

/// Spotify prepends a custom header (with normalisation data) to its Ogg files.
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

// Field numbers of the genre lists that Spotify's metadata used to carry.
const ARTIST_GENRE_FIELD: u32 = 9;
const ALBUM_GENRE_FIELD: u32 = 8;

// ---------------------------------------------------------------------------
// Credentials, stored in the format used by spotify-ripper 3.x
// ---------------------------------------------------------------------------

pub fn credentials_path() -> PathBuf {
    settings_dir().join("credentials.json")
}

pub fn has_stored_credentials() -> bool {
    fs::metadata(credentials_path()).is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn load_credentials() -> Result<Credentials> {
    let data: Value = serde_json::from_str(&fs::read_to_string(credentials_path())?)?;
    let field = |names: &[&str]| names.iter().find_map(|n| data.get(*n));

    let username = field(&["username"])
        .and_then(Value::as_str)
        .map(str::to_owned);
    let auth_data = field(&["credentials", "auth_data", "encoded_auth_blob"])
        .and_then(Value::as_str)
        .ok_or("credentials.json has no credentials")?;
    let auth_data = BASE64.decode(auth_data)?;
    let auth_type = match field(&["type", "auth_type"]) {
        Some(Value::String(s)) => {
            protobuf::Enum::from_str(s).ok_or_else(|| format!("unknown credentials type {s}"))?
        }
        Some(Value::Number(n)) => n
            .as_i64()
            .and_then(|n| protobuf::Enum::from_i32(n as i32))
            .ok_or_else(|| format!("unknown credentials type {n}"))?,
        _ => AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
    };

    Ok(Credentials {
        username,
        auth_type,
        auth_data,
    })
}

fn save_credentials(username: &str, auth_data: &[u8]) -> Result<()> {
    let path = credentials_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let data = json!({
        "username": username,
        "credentials": BASE64.encode(auth_data),
        "type": "AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS",
    });
    fs::write(&path, to_pretty_json(&data))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Advertise as a Spotify Connect device and wait for the user to select it
/// in the official Spotify app. On success the reusable credentials are saved.
pub async fn login_via_zeroconf(device_name: &str, timeout: Duration) -> Result<bool> {
    let config = SessionConfig::default();
    let mut discovery = Discovery::builder(config.device_id.clone(), config.client_id.clone())
        .name(device_name.to_owned())
        .device_type(DeviceType::Computer)
        .launch()?;

    outln!("Waiting for Spotify Connect login...");
    outln!(
        "Open the official Spotify app, then in the device/speaker picker select '{device_name}'."
    );

    let credentials = tokio::time::timeout(timeout, discovery.next()).await;
    discovery.shutdown().await;

    let Ok(Some(credentials)) = credentials else {
        outln!("Timed out waiting for Spotify Connect login.");
        return Ok(false);
    };

    // Log in once to exchange the Zeroconf blob for reusable credentials.
    let session = Session::new(config, None);
    session.connect(credentials, false).await?;
    let username = session.username();
    outln!("Paired with Spotify account: {username}");
    save_credentials(&username, &session.auth_data())?;
    session.shutdown();
    Ok(true)
}

// ---------------------------------------------------------------------------
// Metadata model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ArtistRef {
    pub name: String,
    pub id: String,
}

#[derive(Debug)]
pub struct AlbumInfo {
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub year: i32,
    /// (image size, file id): 0 = default, 1 = small, 2 = large, 3 = xlarge.
    pub covers: Vec<(i32, FileId)>,
    /// (disc number, track ids) in album order.
    pub discs: Vec<(i32, Vec<SpotifyUri>)>,
    pub copyrights: Vec<String>,
    pub genres: Vec<String>,
}

impl AlbumInfo {
    pub fn artist_name(&self) -> String {
        self.artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default()
    }

    pub fn num_discs(&self) -> i32 {
        self.discs.iter().map(|(n, _)| *n).max().unwrap_or(0)
    }

    pub fn num_tracks(&self, disc: i32) -> usize {
        self.discs
            .iter()
            .filter(|(n, _)| *n == disc)
            .map(|(_, t)| t.len())
            .sum()
    }

    /// Cover URL, large (640x640) or extra large if `xlarge`.
    pub fn cover_url(&self, xlarge: bool) -> Option<String> {
        let target = if xlarge { 3 } else { 2 };
        self.covers
            .iter()
            .find(|(size, _)| *size == target)
            .or_else(|| self.covers.iter().max_by_key(|(size, _)| *size))
            .map(|(_, id)| format!("{IMAGE_URL}{}", id.to_base16()))
    }
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub id: String,
    pub name: String,
    /// Duration in milliseconds.
    pub duration: u32,
    pub number: i32,
    pub disc: i32,
    pub artists: Vec<ArtistRef>,
    pub album: Arc<AlbumInfo>,
}

impl TrackInfo {
    pub fn uri(&self) -> String {
        format!("spotify:track:{}", self.id)
    }

    pub fn artist_name(&self) -> String {
        self.artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default()
    }

    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug)]
pub struct ArtistInfo {
    pub genres: Vec<String>,
    /// Current release of every album, keyed by "album", "single",
    /// "compilation" and "appears_on".
    pub albums: HashMap<&'static str, Vec<String>>,
}

#[derive(Debug)]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub tracks: Vec<String>,
    /// Track URI -> (time added as a Unix timestamp, user who added it).
    pub added: HashMap<String, (i64, String)>,
}

fn base62(uri: &SpotifyUri) -> String {
    SpotifyId::try_from(uri)
        .map(|id| id.to_base62())
        .unwrap_or_default()
}

fn artist_refs<'a>(artists: impl Iterator<Item = (&'a SpotifyUri, &'a String)>) -> Vec<ArtistRef> {
    artists
        .map(|(id, name)| ArtistRef {
            name: name.clone(),
            id: base62(id),
        })
        .collect()
}

fn unknown_strings(fields: &protobuf::UnknownFields, number: u32) -> Vec<String> {
    fields
        .iter()
        .filter(|(n, _)| *n == number)
        .filter_map(|(_, v)| match v {
            UnknownValueRef::LengthDelimited(b) => String::from_utf8(b.to_vec()).ok(),
            _ => None,
        })
        .collect()
}

pub fn parse_id(kind: &str, id: &str) -> Result<SpotifyUri> {
    let id = SpotifyId::from_base62(id).map_err(|_| format!("invalid Spotify id {id}"))?;
    Ok(match kind {
        "track" => SpotifyUri::Track { id },
        "album" => SpotifyUri::Album { id },
        "artist" => SpotifyUri::Artist { id },
        "playlist" => SpotifyUri::Playlist { user: None, id },
        _ => return Err(format!("unsupported Spotify URI type {kind}").into()),
    })
}

// ---------------------------------------------------------------------------
// Session, metadata and audio access
// ---------------------------------------------------------------------------

pub struct Spotify {
    session: Session,
    albums: Mutex<HashMap<String, Arc<AlbumInfo>>>,
    artists: Mutex<HashMap<String, Arc<ArtistInfo>>>,
}

pub struct AudioStream {
    pub reader: AudioDecrypt<AudioFile>,
    /// Size of the Ogg Vorbis data, without Spotify's header.
    pub size: u64,
}

impl Spotify {
    /// Create a session from the stored credentials file, refreshing it with
    /// the new reusable credentials returned by Spotify.
    pub async fn connect() -> Result<Self> {
        let credentials = load_credentials()?;
        let cache = Cache::new(None, None, Some(cache_dir()), None)?;
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(credentials, false).await?;
        save_credentials(&session.username(), &session.auth_data())?;
        Ok(Self {
            session,
            albums: Mutex::default(),
            artists: Mutex::default(),
        })
    }

    pub fn username(&self) -> String {
        self.session.username()
    }

    pub fn account_type(&self) -> Option<String> {
        self.session.get_user_attribute("type")
    }

    pub fn shutdown(&self) {
        self.session.shutdown();
    }

    pub async fn track(&self, id: &str) -> Result<TrackInfo> {
        let uri = parse_id("track", id)?;
        let track = Track::get(&self.session, &uri).await?;
        let album = self.album(&base62(&track.album.id)).await?;
        Ok(TrackInfo {
            id: id.to_owned(),
            name: track.name,
            duration: track.duration.max(0) as u32,
            number: track.number,
            disc: track.disc_number,
            artists: artist_refs(track.artists.iter().map(|a| (&a.id, &a.name))),
            album,
        })
    }

    pub async fn album(&self, id: &str) -> Result<Arc<AlbumInfo>> {
        if let Some(album) = self.albums.lock().unwrap().get(id) {
            return Ok(album.clone());
        }
        let uri = parse_id("album", id)?;
        let response = Album::request(&self.session, &uri).await?;
        let msg = <Album as Metadata>::Message::parse_from_bytes(&response)?;
        let genres = unknown_strings(msg.special_fields.unknown_fields(), ALBUM_GENRE_FIELD);
        let album = Album::parse(&msg, &uri)?;

        let images = if album.cover_group.is_empty() {
            &album.covers
        } else {
            &album.cover_group
        };
        let info = Arc::new(AlbumInfo {
            name: album.name.clone(),
            artists: artist_refs(album.artists.iter().map(|a| (&a.id, &a.name))),
            year: album.date.as_utc().year(),
            covers: images.iter().map(|i| (i.size as i32, i.id)).collect(),
            discs: album
                .discs
                .iter()
                .map(|d| (d.number.max(1), d.tracks.0.clone()))
                .collect(),
            copyrights: album.copyrights.iter().map(|c| c.text.clone()).collect(),
            genres,
        });
        self.albums
            .lock()
            .unwrap()
            .insert(id.to_owned(), info.clone());
        Ok(info)
    }

    pub async fn artist(&self, id: &str) -> Result<Arc<ArtistInfo>> {
        if let Some(artist) = self.artists.lock().unwrap().get(id) {
            return Ok(artist.clone());
        }
        let uri = parse_id("artist", id)?;
        let response = Artist::request(&self.session, &uri).await?;
        let msg = <Artist as Metadata>::Message::parse_from_bytes(&response)?;
        let genres = unknown_strings(msg.special_fields.unknown_fields(), ARTIST_GENRE_FIELD);
        let artist = Artist::parse(&msg, &uri)?;

        let ids = |groups: &librespot_metadata::artist::AlbumGroups| {
            groups.current_releases().map(base62).collect::<Vec<_>>()
        };
        let info = Arc::new(ArtistInfo {
            genres,
            albums: HashMap::from([
                ("album", ids(&artist.albums)),
                ("single", ids(&artist.singles)),
                ("compilation", ids(&artist.compilations)),
                ("appears_on", ids(&artist.appears_on_albums)),
            ]),
        });
        self.artists
            .lock()
            .unwrap()
            .insert(id.to_owned(), info.clone());
        Ok(info)
    }

    pub async fn playlist(&self, id: &str) -> Result<PlaylistInfo> {
        let uri = parse_id("playlist", id)?;
        let response = Playlist::request(&self.session, &uri).await?;
        let msg = <Playlist as Metadata>::Message::parse_from_bytes(&response)?;
        let owner = msg.owner_username().to_owned();
        let playlist = Playlist::parse(&msg, &uri)?;

        let mut tracks = Vec::new();
        let mut added = HashMap::new();
        for item in playlist.contents.items.iter() {
            let SpotifyUri::Track { .. } = item.id else {
                continue;
            };
            let track_uri = item.id.to_uri();
            added.insert(
                track_uri.clone(),
                (
                    item.attributes.timestamp.as_timestamp_ms() / 1000,
                    item.attributes.added_by.clone(),
                ),
            );
            tracks.push(track_uri);
        }

        Ok(PlaylistInfo {
            id: id.to_owned(),
            name: playlist.attributes.name.clone(),
            owner,
            tracks,
            added,
        })
    }

    /// Download an image, e.g. a cover from the image CDN.
    pub async fn fetch_url(&self, url: &str) -> Result<Bytes> {
        let request = http::Request::get(url).body(Bytes::new())?;
        Ok(self.session.http_client().request_body(request).await?)
    }

    /// Find the Ogg Vorbis file to download for a track, preferring
    /// `kbps` and falling back to the other bitrates or alternative tracks.
    pub async fn audio_file(&self, id: &str, kbps: u32) -> Result<(SpotifyId, FileId, u32)> {
        let uri = parse_id("track", id)?;
        let mut item = AudioItem::get_file(&self.session, uri).await?;
        if let Err(reason) = &item.availability {
            return Err(format!("track is unavailable: {reason:?}").into());
        }
        if item.files.is_empty()
            && let Some(alternatives) = item.alternatives.take()
        {
            for alt in alternatives.0 {
                if let Ok(alt_item) = AudioItem::get_file(&self.session, alt).await
                    && alt_item.availability.is_ok()
                    && !alt_item.files.is_empty()
                {
                    item = alt_item;
                    break;
                }
            }
        }

        log::debug!(
            "{}: files {:?}, alternatives {:?}",
            item.uri,
            item.files.keys().collect::<Vec<_>>(),
            item.alternatives
        );

        let mut bitrates = vec![kbps];
        bitrates.extend([320, 160, 96].into_iter().filter(|b| *b != kbps));
        for bitrate in bitrates {
            let format = match bitrate {
                320 => AudioFileFormat::OGG_VORBIS_320,
                160 => AudioFileFormat::OGG_VORBIS_160,
                _ => AudioFileFormat::OGG_VORBIS_96,
            };
            if let Some(file_id) = item.files.get(&format) {
                let track_id = SpotifyId::try_from(&item.track_id)?;
                return Ok((track_id, *file_id, bitrate));
            }
        }
        if item.files.is_empty() {
            return Err("Spotify provides no audio files for this track".into());
        }
        Err("track is not available in Ogg Vorbis format".into())
    }

    pub async fn audio_key(
        &self,
        track_id: SpotifyId,
        file_id: FileId,
    ) -> Result<librespot_core::audio_key::AudioKey> {
        Ok(self.session.audio_key().request(track_id, file_id).await?)
    }

    pub async fn open_audio(
        &self,
        file_id: FileId,
        kbps: u32,
        key: librespot_core::audio_key::AudioKey,
    ) -> Result<AudioStream> {
        // A high data rate makes the loader fetch far ahead of the reader,
        // there is no playback to keep pace with.
        let bytes_per_second = kbps as usize * 1024 / 8 * 30;
        let file = AudioFile::open(&self.session, file_id, bytes_per_second).await?;
        let controller = file.get_stream_loader_controller()?;
        controller.set_stream_mode();
        let total = controller.len() as u64;

        let mut reader = AudioDecrypt::new(Some(key), file);
        reader.seek(SeekFrom::Start(SPOTIFY_OGG_HEADER_END))?;
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"OggS" {
            return Err("downloaded stream is not Ogg Vorbis (decryption failed?)".into());
        }
        reader.seek(SeekFrom::Start(SPOTIFY_OGG_HEADER_END))?;

        Ok(AudioStream {
            reader,
            size: total.saturating_sub(SPOTIFY_OGG_HEADER_END),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_cover_and_disc_helpers() {
        let album = AlbumInfo {
            name: "A".into(),
            artists: vec![],
            year: 2020,
            covers: vec![
                (1, FileId([1; 20])),
                (2, FileId([2; 20])),
                (3, FileId([3; 20])),
            ],
            discs: vec![(1, vec![]), (2, vec![])],
            copyrights: vec![],
            genres: vec![],
        };
        assert!(album.cover_url(false).unwrap().ends_with(&"02".repeat(20)));
        assert!(album.cover_url(true).unwrap().ends_with(&"03".repeat(20)));
        assert_eq!(album.num_discs(), 2);
    }
}
