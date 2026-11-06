mod assets;
mod playback;
mod source;

#[cfg(test)]
mod tests;

pub use assets::{SoundAssetError, SoundLibrary, SoundParameters};
pub(crate) use playback::SoundRuntime;
pub use source::{AudioEvent, RealtimeAudio};

#[cfg(test)]
use assets::{DecodedSound, pan_gains};
#[cfg(test)]
use assets::{SoundEntry, decode_sound_asset, scratch_parse_int};
#[cfg(test)]
use source::{PannedSound, PlaybackControls};
