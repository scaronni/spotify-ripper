# librespot's generated protobuf modules require protobuf's pure-Python
# implementation (with the C/upb implementation they raise "Descriptors cannot
# be created directly" on protobuf >= 3.21).  Select it before anything imports
# librespot.  The protobuf traffic here (handshake + metadata) is tiny, so the
# speed cost is irrelevant.  An explicit override in the environment wins.
import os
os.environ.setdefault("PROTOCOL_BUFFERS_PYTHON_IMPLEMENTATION", "python")
