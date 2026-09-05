//! Audio file type plugin: core and presentation halves.
//!
//! One plugin covers the common audio codecs (MP3, FLAC, WAV, Ogg
//! Vorbis/Opus, M4A) via `lofty`, matching how the existing `image` plugin
//! covers multiple raster codecs with one crate. The view is metadata only
//! (format, duration, sample rate, channels, bitrate, tags), not decoded
//! audio: per `plugin-api`'s presentation half, a front end gets lines of
//! text, not samples to play back.

use lofty::file::FileType;
use lofty::prelude::*;
use lofty::probe::Probe;
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io;
use std::path::Path;

/// Whether `prefix` starts with an MP3 frame sync (`0xFF` followed by three
/// set high bits) or an `ID3v2` tag header.
fn is_mp3(prefix: &[u8]) -> bool {
    prefix.starts_with(b"ID3")
        || (prefix.len() >= 2 && prefix[0] == 0xFF && (prefix[1] & 0xE0) == 0xE0)
}

/// Whether `prefix` is a WAV file: a RIFF container declaring a WAVE form.
fn is_wav(prefix: &[u8]) -> bool {
    prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"WAVE"
}

/// Whether `prefix` is an MPEG-4 container (`.m4a`, `.m4b`, ...): an `ftyp`
/// box after the leading box-size field.
fn is_mp4(prefix: &[u8]) -> bool {
    prefix.len() >= 8 && &prefix[4..8] == b"ftyp"
}

/// View data produced by [`AudioCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioView {
    /// The detected audio format, e.g. `"Mpeg"`, `"Flac"`, `"Mp4"`.
    pub format: String,
    /// Duration in seconds.
    pub duration_secs: f64,
    /// Sample rate in Hz, if known.
    pub sample_rate: Option<u32>,
    /// Channel count, if known.
    pub channels: Option<u8>,
    /// Audio bitrate in kbps, if known.
    pub bitrate_kbps: Option<u32>,
    /// Track title, from the primary tag, if present.
    pub title: Option<String>,
    /// Track artist, from the primary tag, if present.
    pub artist: Option<String>,
    /// Album title, from the primary tag, if present.
    pub album: Option<String>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The audio plugin's core half.
#[derive(Debug, Default)]
pub struct AudioCore;

impl PluginCore for AudioCore {
    fn name(&self) -> &'static str {
        "audio"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        is_mp3(prefix)
            || is_wav(prefix)
            || is_mp4(prefix)
            || prefix.starts_with(b"fLaC")
            || prefix.starts_with(b"OggS")
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let tagged_file = Probe::open(path)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
            .read()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let format = match tagged_file.file_type() {
            FileType::Custom(name) => name.to_owned(),
            other => format!("{other:?}"),
        };
        let properties = tagged_file.properties();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        let view = AudioView {
            format,
            duration_secs: properties.duration().as_secs_f64(),
            sample_rate: properties.sample_rate(),
            channels: properties.channels(),
            bitrate_kbps: properties.audio_bitrate(),
            title: tag.and_then(Accessor::title).map(Cow::into_owned),
            artist: tag.and_then(Accessor::artist).map(Cow::into_owned),
            album: tag.and_then(Accessor::album).map(Cow::into_owned),
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The audio plugin's presentation half.
#[derive(Debug, Default)]
pub struct AudioPresentation;

impl PluginPresentation for AudioPresentation {
    fn name(&self) -> &'static str {
        "audio"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: AudioView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![
            format!("{} audio", view.format),
            format!("{:.1} seconds", view.duration_secs),
        ];
        if let Some(sample_rate) = view.sample_rate {
            lines.push(format!("{sample_rate} Hz"));
        }
        if let Some(channels) = view.channels {
            lines.push(format!("{channels} channels"));
        }
        if let Some(bitrate) = view.bitrate_kbps {
            lines.push(format!("{bitrate} kbps"));
        }
        if let Some(title) = &view.title {
            lines.push(format!("Title: {title}"));
        }
        if let Some(artist) = &view.artist {
            lines.push(format!("Artist: {artist}"));
        }
        if let Some(album) = &view.album {
            lines.push(format!("Album: {album}"));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioCore, AudioPresentation, AudioView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-audio-test-{}-{name}",
            std::process::id()
        ))
    }

    /// A minimal, valid FLAC file: magic, a `STREAMINFO` block marked last,
    /// and no audio frames - enough for `lofty` to parse properties.
    fn write_test_flac(path: &std::path::Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"fLaC");
        // STREAMINFO block header: last-metadata-block flag set, type 0, 34-byte length.
        bytes.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]);
        // min/max blocksize (16 bits each).
        bytes.extend_from_slice(&[0x10, 0x00]);
        bytes.extend_from_slice(&[0x10, 0x00]);
        // min/max framesize (24 bits each).
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]);
        // Sample rate (20 bits) = 44100, channels-1 (3 bits) = 1 (stereo),
        // bits-per-sample-1 (5 bits) = 15 (16-bit), total samples (36 bits) = 44100.
        bytes.extend_from_slice(&[0x0A, 0xC4, 0x42, 0xF0, 0x00, 0x00, 0xAC, 0x44]);
        // MD5 signature (16 bytes).
        bytes.extend_from_slice(&[0u8; 16]);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn sniffs_a_real_flac_header() {
        let flac_magic = b"fLaC";
        assert!(AudioCore.sniff(flac_magic));
        assert!(!AudioCore.sniff(b"not audio"));
    }

    #[test]
    fn sniffs_an_mp3_by_id3_tag() {
        assert!(AudioCore.sniff(b"ID3\x04\x00\x00\x00\x00\x00\x00"));
    }

    #[test]
    fn sniffs_an_mp3_by_frame_sync() {
        assert!(AudioCore.sniff(&[0xFF, 0xFB, 0x90, 0x00]));
    }

    #[test]
    fn sniffs_a_wav_header() {
        let mut prefix = Vec::from(*b"RIFF");
        prefix.extend_from_slice(&[0, 0, 0, 0]);
        prefix.extend_from_slice(b"WAVE");
        assert!(AudioCore.sniff(&prefix));
    }

    #[test]
    fn sniffs_an_m4a_header() {
        let mut prefix = vec![0, 0, 0, 0x18];
        prefix.extend_from_slice(b"ftypM4A ");
        assert!(AudioCore.sniff(&prefix));
    }

    #[test]
    fn does_not_sniff_a_plain_riff_that_is_not_wave() {
        let mut prefix = Vec::from(*b"RIFF");
        prefix.extend_from_slice(&[0, 0, 0, 0]);
        prefix.extend_from_slice(b"AVI ");
        assert!(!AudioCore.sniff(&prefix));
    }

    #[test]
    fn views_a_real_flac_file_for_sample_rate_and_channels() {
        let path = unique_temp_file("test.flac");
        write_test_flac(&path);

        let data = AudioCore.view(&path).unwrap();
        let view: AudioView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "Flac");
        assert_eq!(view.sample_rate, Some(44100));
        assert_eq!(view.channels, Some(2));
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_format_and_playback_metadata() {
        let data = serde_json::to_value(AudioView {
            format: "Flac".to_owned(),
            duration_secs: 1.0,
            sample_rate: Some(44100),
            channels: Some(2),
            bitrate_kbps: Some(320),
            title: Some("Song".to_owned()),
            artist: None,
            album: None,
            file_size: 123,
        })
        .unwrap();

        let lines = AudioPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Flac audio",
                "1.0 seconds",
                "44100 Hz",
                "2 channels",
                "320 kbps",
                "Title: Song",
                "123 bytes on disk",
            ]
        );
    }
}
