//! Playback codecs: Symphonia's built-ins plus bundled libopus.
use std::sync::{Mutex, OnceLock};
use symphonia::core::audio::{AsAudioBufferRef, AudioBuffer, AudioBufferRef, Signal, SignalSpec};
use symphonia::core::codecs::{
    CodecDescriptor, CodecParameters, CodecRegistry, Decoder, DecoderOptions, FinalizeResult,
    CODEC_TYPE_OPUS,
};
use symphonia::core::errors::{decode_error, unsupported_error, Error, Result};
use symphonia::core::formats::Packet;
use symphonia::core::support_codec;

pub(crate) fn registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_all::<OpusDecoder>();
        registry
    })
}

const OPUS_RATE: u32 = 48_000;
// An Opus packet may contain up to 120 ms of audio.
const MAX_FRAMES: usize = 5_760;

/// Discard decoded pre-roll and clip the first/last Opus packet to the requested interval.
/// Packet timestamps are in 48 kHz audible samples after the demuxer's gapless trimming.
pub(crate) fn trim_opus_output(
    output: &mut AudioBufferRef<'_>,
    packet_start: u64,
    start_frame: u64,
    end_frame: Option<u64>,
) {
    let frames = output.frames();
    let start = start_frame.saturating_sub(packet_start).min(frames as u64) as usize;
    let end = end_frame.map_or(0, |end| {
        (packet_start + frames as u64)
            .saturating_sub(end)
            .min((frames - start) as u64) as usize
    });
    if start > 0 || end > 0 {
        if let AudioBufferRef::F32(buffer) = output {
            buffer.to_mut().trim(start, end);
        }
    }
}

struct OpusDecoder {
    params: CodecParameters,
    decoder: Mutex<opus::MSDecoder>,
    pcm: Vec<f32>,
    buffer: AudioBuffer<f32>,
    channels: usize,
}

struct Header {
    streams: u8,
    coupled: u8,
    mapping: Vec<u8>,
    gain: i16,
}

fn parse_header(params: &CodecParameters) -> Result<Header> {
    let data = params
        .extra_data
        .as_deref()
        .ok_or(Error::DecodeError("opus: missing OpusHead"))?;
    if data.len() < 19 || &data[..8] != b"OpusHead" || data[8] > 15 {
        return decode_error("opus: invalid OpusHead");
    }
    let channels = usize::from(data[9]);
    if channels == 0 || channels > 8 || params.channels.map(|c| c.count()) != Some(channels) {
        return unsupported_error("opus: unsupported channel layout");
    }
    let gain = i16::from_le_bytes([data[16], data[17]]);
    let (streams, coupled, mapping) = match data[18] {
        0 if channels <= 2 => (1, (channels - 1) as u8, (0..channels as u8).collect()),
        1 if data.len() >= 21 + channels => {
            let streams = data[19];
            let coupled = data[20];
            if streams == 0
                || coupled > streams
                || usize::from(streams) + usize::from(coupled) > 255
            {
                return decode_error("opus: invalid stream counts");
            }
            let input = &data[21..21 + channels];
            if input
                .iter()
                .any(|&c| c != 255 && usize::from(c) >= usize::from(streams) + usize::from(coupled))
            {
                return decode_error("opus: invalid channel mapping");
            }
            // Opus family 1 uses Vorbis order; Symphonia uses the channel bitmask order.
            let order: &[usize] = match channels {
                1 => &[0],
                2 => &[0, 1],
                3 => &[0, 2, 1],
                4 => &[0, 1, 2, 3],
                5 => &[0, 2, 1, 3, 4],
                6 => &[0, 2, 1, 5, 3, 4],
                7 => &[0, 2, 1, 6, 5, 3, 4],
                8 => &[0, 2, 1, 7, 5, 6, 3, 4],
                _ => unreachable!(),
            };
            (streams, coupled, order.iter().map(|&i| input[i]).collect())
        }
        _ => return unsupported_error("opus: unsupported channel mapping family"),
    };
    Ok(Header {
        streams,
        coupled,
        mapping,
        gain,
    })
}

impl Decoder for OpusDecoder {
    fn try_new(params: &CodecParameters, _: &DecoderOptions) -> Result<Self> {
        if params.codec != CODEC_TYPE_OPUS || params.sample_rate != Some(OPUS_RATE) {
            return unsupported_error("opus: expected Opus at 48000 Hz");
        }
        let header = parse_header(params)?;
        let channels = header.mapping.len();
        let mut decoder =
            opus::MSDecoder::new(OPUS_RATE, header.streams, header.coupled, &header.mapping)
                .map_err(|_| Error::DecodeError("opus: could not create decoder"))?;
        decoder
            .set_gain(i32::from(header.gain))
            .map_err(|_| Error::DecodeError("opus: invalid gain"))?;
        let spec = SignalSpec::new(OPUS_RATE, params.channels.unwrap());
        Ok(Self {
            params: params.clone(),
            decoder: Mutex::new(decoder),
            pcm: vec![0.0; MAX_FRAMES * channels],
            buffer: AudioBuffer::new(MAX_FRAMES as u64, spec),
            channels,
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(CODEC_TYPE_OPUS, "opus", "Opus (libopus)")]
    }

    fn reset(&mut self) {
        if let Ok(decoder) = self.decoder.get_mut() {
            let _ = decoder.reset_state();
        }
        self.buffer.clear();
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        self.buffer.clear();
        if packet.data.is_empty() {
            return decode_error("opus: empty packet");
        }
        let frames = self
            .decoder
            .get_mut()
            .map_err(|_| Error::DecodeError("opus: decoder lock poisoned"))?
            .decode_float(&packet.data, &mut self.pcm, false)
            .map_err(|_| Error::DecodeError("opus: invalid audio packet"))?;
        self.buffer.render_reserved(Some(frames));
        for channel in 0..self.channels {
            let output = self.buffer.chan_mut(channel);
            for (frame, sample) in output.iter_mut().enumerate() {
                *sample = self.pcm[frame * self.channels + channel];
            }
        }
        self.buffer
            .trim(packet.trim_start() as usize, packet.trim_end() as usize);
        Ok(self.buffer.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }
    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buffer.as_audio_buffer_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    use symphonia::core::units::Time;

    const STEREO: &[u8] = include_bytes!("../tests/fixtures/opus-stereo.opus");
    const MONO: &[u8] = include_bytes!("../tests/fixtures/opus-mono.opus");
    const LONG: &[u8] = include_bytes!("../tests/fixtures/opus-120ms.opus");
    const SURROUND: &[u8] = include_bytes!("../tests/fixtures/opus-surround.opus");

    fn decode(data: &[u8], seek: f64, end: Option<f64>) -> (CodecParameters, Vec<Vec<f32>>) {
        let source =
            MediaSourceStream::new(Box::new(Cursor::new(data.to_vec())), Default::default());
        let mut format = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                source,
                &FormatOptions {
                    enable_gapless: true,
                    ..FormatOptions::default()
                },
                &MetadataOptions::default(),
            )
            .unwrap()
            .format;
        let track = format.default_track().unwrap();
        let id = track.id;
        let params = track.codec_params.clone();
        let mut decoder = registry()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let rate = params.sample_rate.unwrap();
        let mut output = vec![Vec::new(); params.channels.unwrap().count()];
        if seek > 0.0 {
            let from = (seek - 0.080).max(0.0);
            format
                .seek(
                    SeekMode::Coarse,
                    SeekTo::Time {
                        time: Time::new(from as u64, from.fract()),
                        track_id: Some(id),
                    },
                )
                .unwrap();
            decoder.reset();
        }
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break
                }
                Err(error) => panic!("packet error: {error}"),
            };
            if packet.track_id() != id {
                continue;
            }
            let mut decoded = decoder.decode(&packet).unwrap();
            if params.codec == CODEC_TYPE_OPUS {
                trim_opus_output(
                    &mut decoded,
                    packet.ts(),
                    (seek * f64::from(rate)).round() as u64,
                    end.map(|end| (end * f64::from(rate)).round() as u64),
                );
            }
            let mut samples = AudioBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            decoded.convert(&mut samples);
            for (channel, output) in output.iter_mut().enumerate() {
                output.extend_from_slice(samples.chan(channel));
            }
        }
        (params, output)
    }

    #[test]
    fn opus_mono_and_stereo_decode_at_48khz_without_encoder_padding() {
        for (data, channels, frames) in [(MONO, 1, 23_040), (STEREO, 2, 156_000)] {
            let (params, output) = decode(data, 0.0, None);
            assert_eq!(params.codec, CODEC_TYPE_OPUS);
            assert_eq!(params.sample_rate, Some(48_000));
            assert_eq!(output.len(), channels);
            for samples in output {
                assert_eq!(samples.len(), frames);
                assert!(samples.iter().all(|s| s.is_finite()));
                assert!(samples.iter().map(|s| s * s).sum::<f32>() / frames as f32 > 0.001);
            }
        }
    }

    #[test]
    fn opus_decodes_120ms_packets() {
        let (_, output) = decode(LONG, 0.0, None);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].len(), 24_000);
        assert!(output[0].iter().any(|s| s.abs() > 0.05));
    }

    #[test]
    fn opus_seeking_and_segment_end_are_sample_accurate() {
        let (_, full) = decode(STEREO, 0.0, None);
        let (_, seeked) = decode(STEREO, 1.5, Some(2.0));
        for (channel, samples) in seeked.iter().enumerate() {
            assert_eq!(samples.len(), 24_000);
            // Decoder state converges during the pre-roll; compare after its settling interval.
            let error = samples[4800..]
                .iter()
                .zip(&full[channel][76_800..96_000])
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>()
                / (samples.len() - 4800) as f32;
            assert!(error < 0.002, "seek mismatch: {error}");
        }
    }

    #[test]
    fn opus_surround_channels_follow_symphonia_order() {
        let (_, output) = decode(SURROUND, 0.0, None);
        assert_eq!(output.len(), 6);
        for (samples, expected_hz) in output.iter().zip([220.0, 330.0, 440.0, 55.0, 660.0, 770.0]) {
            assert_eq!(samples.len(), 48_000);
            let middle = &samples[4800..43_200];
            let crossings = middle
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let hz = crossings as f64 / 0.8;
            assert!(
                (hz - expected_hz).abs() < 3.0,
                "channel frequency {hz}, expected {expected_hz}"
            );
        }
    }

    #[test]
    fn builtin_flac_mp3_and_vorbis_decoders_remain_available() {
        for (data, expected_frames) in [
            (
                include_bytes!("../tests/fixtures/reference.flac").as_slice(),
                12_000,
            ),
            (
                include_bytes!("../tests/fixtures/reference.mp3").as_slice(),
                22_050,
            ),
            (
                include_bytes!("../tests/fixtures/reference.ogg").as_slice(),
                // Preserve upstream 0.5's existing single-page Vorbis padding behavior.
                22_528,
            ),
        ] {
            let (params, output) = decode(data, 0.0, None);
            assert_ne!(params.codec, CODEC_TYPE_OPUS);
            assert_eq!(output[0].len(), expected_frames);
        }
    }

    #[test]
    fn rejects_invalid_headers_and_resets_after_bad_packets() {
        let mut params = CodecParameters::new();
        params
            .for_codec(CODEC_TYPE_OPUS)
            .with_sample_rate(48_000)
            .with_channels(symphonia::core::audio::Layout::Stereo.into_channels());
        assert!(registry()
            .make(&params, &DecoderOptions::default())
            .is_err());
        let mut header = b"OpusHead\x01\x02\x00\x00\x80\xbb\x00\x00\x00\x00\x00".to_vec();
        params.with_extra_data(header.clone().into_boxed_slice());
        let mut decoder = registry()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        assert!(decoder
            .decode(&Packet::new_from_slice(0, 0, 0, &[]))
            .is_err());
        decoder.reset();
        header[9] = 0;
        params.with_extra_data(header.into_boxed_slice());
        assert!(registry()
            .make(&params, &DecoderOptions::default())
            .is_err());
    }

    #[test]
    fn applies_opus_header_gain_and_recovers_after_reset() {
        let source =
            MediaSourceStream::new(Box::new(Cursor::new(STEREO.to_vec())), Default::default());
        let mut format = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                source,
                &FormatOptions {
                    enable_gapless: true,
                    ..FormatOptions::default()
                },
                &MetadataOptions::default(),
            )
            .unwrap()
            .format;
        let mut params = format.default_track().unwrap().codec_params.clone();
        let packet = format.next_packet().unwrap();
        let mut decoder = registry()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let original = match decoder.decode(&packet).unwrap() {
            AudioBufferRef::F32(buffer) => buffer.chan(0).to_vec(),
            _ => panic!("expected floating point Opus samples"),
        };
        assert!(decoder
            .decode(&Packet::new_from_slice(0, 0, 0, &[]))
            .is_err());
        decoder.reset();
        let reset = match decoder.decode(&packet).unwrap() {
            AudioBufferRef::F32(buffer) => buffer.chan(0).to_vec(),
            _ => unreachable!(),
        };
        assert_eq!(original, reset);

        let mut header = params.extra_data.take().unwrap().into_vec();
        header[16..18].copy_from_slice(&(6_i16 * 256).to_le_bytes());
        params.with_extra_data(header.into_boxed_slice());
        let mut louder = registry()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let decoded = louder.decode(&packet).unwrap();
        let AudioBufferRef::F32(buffer) = decoded else {
            unreachable!()
        };
        let gain = 10_f32.powf(6.0 / 20.0);
        for (a, b) in original.iter().zip(buffer.chan(0)) {
            assert!((a * gain - b).abs() < 0.0001);
        }
    }
}
