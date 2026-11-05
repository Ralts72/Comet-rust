//! Versioned interleaved PCM, independent of codecs and audio devices.
use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, sync::Arc};

pub const AUDIO_VERSION: u32 = 1;
pub const MAX_AUDIO_SAMPLES: usize = 8_388_608;
pub const MAX_AUDIO_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioData {
    pub version: u32,
    pub sample_rate: u32,
    pub channels: u16,
    #[serde(deserialize_with = "samples")]
    pub samples: Arc<[i16]>,
}
impl AudioData {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == AUDIO_VERSION, "unsupported audio version");
        ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "audio sample rate must be between 8000 and 192000 Hz"
        );
        ensure!(
            matches!(self.channels, 1 | 2),
            "audio requires one or two channels"
        );
        ensure!(
            !self.samples.is_empty() && self.samples.len().is_multiple_of(self.channels as usize),
            "audio requires nonempty complete interleaved frames"
        );
        ensure!(
            self.samples.len() <= MAX_AUDIO_SAMPLES,
            "audio exceeds its sample budget"
        );
        // An i16 needs at most six decimal characters plus a separator in compact JSON.
        ensure!(
            self.samples.len() * 7 + 128 <= MAX_AUDIO_BYTES,
            "audio exceeds its encoded size budget"
        );
        Ok(())
    }
}

fn samples<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<[i16]>, D::Error> {
    struct Samples;
    impl<'de> de::Visitor<'de> for Samples {
        type Value = Arc<[i16]>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a bounded array of interleaved PCM16 samples")
        }
        fn visit_seq<S: de::SeqAccess<'de>>(
            self,
            mut sequence: S,
        ) -> Result<Self::Value, S::Error> {
            if sequence
                .size_hint()
                .is_some_and(|size| size > MAX_AUDIO_SAMPLES)
            {
                return Err(de::Error::custom("audio exceeds its sample budget"));
            }
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                if values.len() == MAX_AUDIO_SAMPLES {
                    return Err(de::Error::custom("audio exceeds its sample budget"));
                }
                values.push(value);
            }
            Ok(values.into())
        }
    }
    deserializer.deserialize_seq(Samples)
}
