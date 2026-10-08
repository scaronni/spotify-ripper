// Metadata tags and cover art, plus the per-track summary printed after a rip.

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemValue, Tag, TagItem, TagType};
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::args::args;
use crate::format::{Context, format_track_string};
use crate::outln;
use crate::output::{RESET, YELLOW, format_field};
use crate::spotify::TrackInfo;
use crate::util::{format_size, format_time, to_ascii_with};

/// Duration of an existing audio file, if it can be read.
pub fn file_duration(path: &Path) -> Option<Duration> {
    let tagged = lofty::read_from_path(path).ok()?;
    Some(tagged.properties().duration())
}

pub fn set_metadata_tags(
    ctx: &Context,
    audio_file: &Path,
    idx: usize,
    track: &TrackInfo,
    image: Option<Vec<u8>>,
    indent: &str,
) {
    let a = args();
    if a.output_type == "wav" || a.output_type == "pcm" {
        outln!(
            "{YELLOW}Skipping metadata tagging for {} encoding...{RESET}",
            a.output_type
        );
        return;
    }

    let num_discs = track.album.num_discs();
    let num_tracks = track.album.num_tracks(track.disc);

    // with --ascii-path-only the tags keep their unicode characters
    let replace = a.ascii_path_only;
    let tag_str = |s: &str| {
        if a.ascii_path_only {
            s.to_owned()
        } else {
            to_ascii_with(s, replace)
        }
    };

    let artists = if a.all_artists {
        track.artist_names()
    } else {
        track.artist_name()
    };
    let album = track.album.name.clone();
    let album_artist = track.album.artist_name();
    let title = track.name.clone();
    let comment = a
        .comment
        .as_ref()
        .map(|c| format_track_string(ctx, c, idx, track));
    let grouping = a
        .grouping
        .as_ref()
        .map(|g| format_track_string(ctx, g, idx, track));
    let year = track.album.year.to_string();

    // write the cover image to a file if requested, and decide on embedding
    let mut embed_image = None;
    if let Some(image) = image {
        let write_image = |name: &str| {
            let cover = audio_file.parent().unwrap_or(Path::new(".")).join(name);
            if !cover.exists()
                && let Err(e) = fs::write(&cover, &image)
            {
                outln!("{YELLOW}Warning: could not save cover image: {e}{RESET}");
            }
        };
        if let Some(name) = &a.cover_file {
            write_image(name);
        } else if let Some(name) = &a.cover_file_and_embed {
            write_image(name);
            embed_image = Some(image);
        } else {
            embed_image = Some(image);
        }
    }

    let result = (|| -> lofty::error::Result<Duration> {
        let mut tagged = lofty::read_from_path(audio_file)?;
        let tag_type = tagged.primary_tag_type();
        if tagged.primary_tag().is_none() {
            tagged.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged.primary_tag_mut().expect("primary tag present");

        let mut set = |key: ItemKey, value: String| {
            tag.insert(TagItem::new(key, ItemValue::Text(value)));
        };
        set(ItemKey::AlbumTitle, tag_str(&album));
        set(ItemKey::TrackTitle, tag_str(&title));
        set(ItemKey::TrackArtist, tag_str(&artists));
        set(ItemKey::AlbumArtist, tag_str(&album_artist));
        set(ItemKey::RecordingDate, year.clone());
        set(ItemKey::DiscNumber, track.disc.to_string());
        set(ItemKey::TrackNumber, track.number.to_string());
        if tag_type == TagType::VorbisComments {
            set(ItemKey::Year, year.clone());
        }
        if num_discs > 0 || tag_type == TagType::VorbisComments {
            set(ItemKey::DiscTotal, num_discs.to_string());
        }
        if num_tracks > 0 || tag_type == TagType::VorbisComments {
            set(ItemKey::TrackTotal, num_tracks.to_string());
        }
        if let Some(comment) = &comment {
            set(ItemKey::Comment, tag_str(comment));
        }
        if let Some(grouping) = &grouping {
            set(ItemKey::ContentGroup, tag_str(grouping));
        }
        if let Some(image) = embed_image {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(
                Picture::unchecked(image)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Jpeg)
                    .description("Front Cover")
                    .build(),
            );
        }

        let mut options = WriteOptions::default();
        if a.id3_v23 {
            options.use_id3v23(true);
        }
        tagged.save_to_path(audio_file, options)?;
        Ok(tagged.properties().duration())
    })();

    let length = match result {
        Ok(length) => length,
        Err(e) => {
            outln!("{YELLOW}Warning: exception while saving tags: {e}{RESET}");
            return;
        }
    };

    // log track metadata, indented to align under the track line
    let field = |label: &str, value: &str| outln!("{}", format_field(indent, label, value));
    field("Artist", &to_ascii_with(&artists, replace));
    field("Album", &to_ascii_with(&album, replace));
    field("Album artist", &to_ascii_with(&album_artist, replace));
    field("Title", &to_ascii_with(&title, replace));
    field("Track number", &format!("{}/{num_tracks}", track.number));
    field("Disc number", &format!("{}/{num_discs}", track.disc));
    field("Year", &year);
    if let Some(comment) = &comment {
        field("Comment", &to_ascii_with(comment, replace));
    }
    if let Some(grouping) = &grouping {
        field("Grouping", &to_ascii_with(grouping, replace));
    }
    field("Time", &format_time(length.as_secs(), None));
    if let Ok(meta) = fs::metadata(audio_file) {
        field("Size", &format_size(meta.len()));
    }
}
