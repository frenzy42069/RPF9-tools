//! rpf9-core: RPF9 container scanning, recovery, and classification.
//!
//! Decryption-less: we recover resources that sit in plain zlib streams or raw
//! media formats; the RPF9-native directory (encrypted) is intentionally out
//! of scope here (see format_desc.md, "Current results").

pub mod archive;
pub mod bytes;
pub mod header;
pub mod index;
pub mod kinds;
pub mod media;
pub mod names;
pub mod report;
pub mod scan;
pub mod stream;
pub mod util;
