use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use libpulse_binding::{
    context::{Context as PulseContext, FlagSet as ContextFlagSet, State as ContextState},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format, Spec},
    stream::{FlagSet as StreamFlagSet, PeekResult, State as StreamState, Stream},
};

pub(crate) const AUDIO_SPECTRUM_BINS: usize = 64;
const AUDIO_FFT_SIZE: usize = 4096;
// Float capture can legitimately exceed unity after per-capture gain. Reserve
// arithmetic headroom for the FFT and response gain only at extreme magnitudes.
const MAX_PCM_MAGNITUDE: f32 = f32::MAX / (AUDIO_FFT_SIZE * AUDIO_FFT_SIZE) as f32;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StereoSpectrum {
    pub(crate) left: [f32; AUDIO_SPECTRUM_BINS],
    pub(crate) right: [f32; AUDIO_SPECTRUM_BINS],
}

impl StereoSpectrum {
    pub(crate) fn flattened(&self) -> Arc<[f32]> {
        let mut values = Vec::with_capacity(AUDIO_SPECTRUM_BINS * 2);
        values.extend_from_slice(&self.left);
        values.extend_from_slice(&self.right);
        values.into()
    }
}

pub(crate) fn spectrum_from_interleaved_pcm(samples: &[f32], sample_rate: u32) -> StereoSpectrum {
    let mut left = [0.0; AUDIO_SPECTRUM_BINS];
    let mut right = [0.0; AUDIO_SPECTRUM_BINS];
    if sample_rate == 0 || samples.len() < 4 {
        return StereoSpectrum { left, right };
    }

    let frame_count = (samples.len() / 2).min(AUDIO_FFT_SIZE);
    let start_frame = samples.len() / 2 - frame_count;
    let mut window_sum = 0.0_f32;
    // Zero-pad shorter capture windows. Work and storage stay bounded even if
    // the caller supplies a backlog; only the latest complete stereo frames count.
    let mut left_fft = vec![[0.0_f32; 2]; AUDIO_FFT_SIZE];
    let mut right_fft = vec![[0.0_f32; 2]; AUDIO_FFT_SIZE];
    for index in 0..frame_count {
        let phase = std::f32::consts::TAU * index as f32 / (frame_count - 1) as f32;
        let weight = 0.5 - 0.5 * phase.cos();
        window_sum += weight;
        for (channel, fft) in [&mut left_fft, &mut right_fft].into_iter().enumerate() {
            let sample = samples[(start_frame + index) * 2 + channel];
            fft[index][0] = if sample.is_finite() {
                sample.clamp(-MAX_PCM_MAGNITUDE, MAX_PCM_MAGNITUDE) * weight
            } else {
                0.0
            };
        }
    }
    if window_sum <= f32::EPSILON {
        return StereoSpectrum { left, right };
    }

    fft_in_place(&mut left_fft);
    fft_in_place(&mut right_fft);

    // Match the native renderer's 20 Hz..20 kHz Mel bands, tilt and dB
    // response. Every FFT bin in a band participates, including off-bin music.
    let sample_rate = sample_rate as f32;
    let max_hz = 20_000.0_f32.min(sample_rate * 0.5);
    let to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let min_mel = to_mel(20.0_f32.min(max_hz));
    let max_mel = to_mel(max_hz);
    let upper_bin = |hz: f32| {
        ((hz * AUDIO_FFT_SIZE as f32 / sample_rate).ceil() as usize).clamp(1, AUDIO_FFT_SIZE / 2)
    };
    let max_bin = upper_bin(max_hz);
    let mut edges = [0; AUDIO_SPECTRUM_BINS + 1];
    edges[0] = upper_bin(20.0_f32.min(max_hz));
    for band in 1..AUDIO_SPECTRUM_BINS {
        let mel = min_mel + (max_mel - min_mel) * band as f32 / AUDIO_SPECTRUM_BINS as f32;
        let hz = 700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0);
        // Reserve a bin for each remaining band at supported capture rates.
        // Degenerate rates may share edges, but must never underflow or reverse.
        let limit = max_bin.saturating_sub(AUDIO_SPECTRUM_BINS - band).max(edges[band - 1]);
        edges[band] = upper_bin(hz).max(edges[band - 1] + 1).min(limit);
    }
    edges[AUDIO_SPECTRUM_BINS] = max_bin;
    for band in 0..AUDIO_SPECTRUM_BINS {
        let begin = edges[band];
        let end = edges[band + 1];
        let center_hz = 0.5 * (begin + end) as f32 * sample_rate / AUDIO_FFT_SIZE as f32;
        let gain = (center_hz / 200.0).powf(0.30);
        for (output, fft) in [(&mut left, &left_fft), (&mut right, &right_fft)] {
            let peak = fft[begin..end]
                .iter()
                .map(|value| value[0].hypot(value[1]))
                .fold(0.0_f32, f32::max);
            let magnitude = peak * (2.0 / window_sum) * gain;
            let db = 20.0 * magnitude.max(1.0e-12).log10();
            let unit = ((db + 80.0) / 72.0).clamp(0.0, 1.0);
            output[band] = if unit <= 0.5 {
                0.5 * (unit * 2.0).powf(1.6)
            } else {
                1.0 - 0.5 * ((1.0 - unit) * 2.0).powf(1.6)
            };
        }
    }

    StereoSpectrum { left, right }
}

// Radix-2 FFT using [real, imaginary] pairs; no extra DSP dependency.
fn fft_in_place(values: &mut [[f32; 2]]) {
    let count = values.len();
    let mut reversed = 0;
    for index in 1..count {
        let mut bit = count >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            values.swap(index, reversed);
        }
    }
    let mut length = 2;
    while length <= count {
        let angle = -std::f32::consts::TAU / length as f32;
        let (step_im, step_re) = angle.sin_cos();
        for begin in (0..count).step_by(length) {
            let (mut weight_re, mut weight_im) = (1.0, 0.0);
            for index in 0..length / 2 {
                let even = values[begin + index];
                let odd = values[begin + index + length / 2];
                let odd_re = odd[0] * weight_re - odd[1] * weight_im;
                let odd_im = odd[0] * weight_im + odd[1] * weight_re;
                values[begin + index] = [even[0] + odd_re, even[1] + odd_im];
                values[begin + index + length / 2] = [even[0] - odd_re, even[1] - odd_im];
                (weight_re, weight_im) = (
                    weight_re * step_re - weight_im * step_im,
                    weight_re * step_im + weight_im * step_re,
                );
            }
        }
        length *= 2;
    }
}

fn append_recent_pcm_bytes(retained: &mut Vec<f32>, bytes: &[u8], max_samples: usize) {
    if max_samples == 0 {
        retained.clear();
        return;
    }

    let chunks = bytes.chunks_exact(std::mem::size_of::<f32>());
    let incoming_samples = chunks.len();
    if incoming_samples >= max_samples {
        retained.clear();
        retained.extend(
            chunks
                .skip(incoming_samples - max_samples)
                .map(|bytes| f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        );
        return;
    }

    let overflow = retained.len().saturating_add(incoming_samples).saturating_sub(max_samples);
    if overflow > 0 {
        retained.drain(..overflow.min(retained.len()));
    }
    retained
        .extend(chunks.map(|bytes| f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])));
}

fn append_recent_silence(retained: &mut Vec<f32>, incoming_samples: usize, max_samples: usize) {
    if max_samples == 0 {
        retained.clear();
        return;
    }
    if incoming_samples >= max_samples {
        retained.clear();
        retained.resize(max_samples, 0.0);
        return;
    }

    let overflow = retained.len().saturating_add(incoming_samples).saturating_sub(max_samples);
    if overflow > 0 {
        retained.drain(..overflow.min(retained.len()));
    }
    retained.resize(retained.len() + incoming_samples, 0.0);
}

pub(crate) struct PulseAudioCapture {
    // Keep the dependency objects in destruction order: stream -> context -> mainloop.
    stream: Stream,
    context: PulseContext,
    mainloop: Mainloop,
    sample_rate: u32,
    frame_count: usize,
    samples: Vec<f32>,
}

impl PulseAudioCapture {
    pub(crate) fn connect(
        source: &str,
        sample_rate: u32,
        update_hz: u32,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>> {
        if cancelled() {
            return Ok(None);
        }
        let sample_rate = sample_rate.clamp(8_000, 192_000);
        let update_hz = update_hz.clamp(5, 60);
        let spec = Spec { format: Format::FLOAT32NE, channels: 2, rate: sample_rate };
        if !spec.is_valid() {
            anyhow::bail!("invalid PulseAudio sample specification");
        }
        let frame_count = (sample_rate / update_hz).clamp(128, 4096) as usize;
        let fragment_bytes = frame_count
            .checked_mul(2)
            .and_then(|samples| samples.checked_mul(std::mem::size_of::<f32>()))
            .and_then(|bytes| u32::try_from(bytes).ok())
            .context("PulseAudio fragment size overflow")?;

        let mut mainloop = Mainloop::new().context("failed to create PulseAudio mainloop")?;
        let mut context = PulseContext::new(&mainloop, "we-layerd")
            .context("failed to create PulseAudio context")?;
        context
            .connect(None, ContextFlagSet::NOFLAGS, None)
            .context("failed to start PulseAudio context connection")?;
        if !drive_pulse_until(&mut mainloop, Duration::from_secs(2), &mut cancelled, || {
            match context.get_state() {
                ContextState::Ready => Ok(true),
                ContextState::Failed | ContextState::Terminated => {
                    Err(anyhow::anyhow!("PulseAudio context failed: {}", context.errno()))
                }
                _ => Ok(false),
            }
        })? {
            return Ok(None);
        }

        let mut stream = Stream::new(&mut context, "Wallpaper audio spectrum", &spec, None)
            .context("failed to create PulseAudio record stream")?;
        let buffer_attr = BufferAttr {
            maxlength: fragment_bytes.saturating_mul(4),
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: fragment_bytes,
        };
        stream
            .connect_record(Some(source), Some(&buffer_attr), StreamFlagSet::ADJUST_LATENCY)
            .context("failed to connect PulseAudio monitor source")?;
        if !drive_pulse_until(
            &mut mainloop,
            Duration::from_secs(2),
            &mut cancelled,
            || match stream.get_state() {
                StreamState::Ready => Ok(true),
                StreamState::Failed | StreamState::Terminated => {
                    Err(anyhow::anyhow!("PulseAudio record stream failed"))
                }
                _ => Ok(false),
            },
        )? {
            return Ok(None);
        }

        Ok(Some(Self {
            stream,
            context,
            mainloop,
            sample_rate,
            frame_count,
            samples: Vec::with_capacity(frame_count * 2),
        }))
    }

    pub(crate) fn poll_spectrum(&mut self) -> Result<Option<Arc<[f32]>>> {
        match self.mainloop.iterate(false) {
            IterateResult::Success(_) => {}
            IterateResult::Quit(_) => anyhow::bail!("PulseAudio mainloop quit"),
            IterateResult::Err(error) => anyhow::bail!("PulseAudio mainloop failed: {error}"),
        }
        match self.context.get_state() {
            ContextState::Failed | ContextState::Terminated => {
                anyhow::bail!("PulseAudio context disconnected: {}", self.context.errno())
            }
            _ => {}
        }
        match self.stream.get_state() {
            StreamState::Failed | StreamState::Terminated => {
                anyhow::bail!("PulseAudio record stream disconnected")
            }
            _ => {}
        }

        let required_samples = self.frame_count * 2;
        loop {
            match self.stream.peek().context("failed to peek PulseAudio samples")? {
                PeekResult::Empty => break,
                PeekResult::Hole(bytes) => append_recent_silence(
                    &mut self.samples,
                    bytes / std::mem::size_of::<f32>(),
                    required_samples,
                ),
                PeekResult::Data(bytes) => {
                    append_recent_pcm_bytes(&mut self.samples, bytes, required_samples)
                }
            }
            self.stream.discard().context("failed to consume PulseAudio samples")?;
        }

        if self.samples.len() < required_samples {
            return Ok(None);
        }
        debug_assert_eq!(self.samples.len(), required_samples);
        let spectrum = spectrum_from_interleaved_pcm(&self.samples, self.sample_rate).flattened();
        self.samples.clear();
        Ok(Some(spectrum))
    }
}

impl Drop for PulseAudioCapture {
    fn drop(&mut self) {
        let _ = self.stream.disconnect();
        self.context.disconnect();
    }
}

fn drive_pulse_until(
    mainloop: &mut Mainloop,
    timeout: Duration,
    cancelled: &mut impl FnMut() -> bool,
    mut ready: impl FnMut() -> Result<bool>,
) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled() {
            return Ok(false);
        }
        match mainloop.iterate(false) {
            IterateResult::Success(_) => {}
            IterateResult::Quit(_) => anyhow::bail!("PulseAudio mainloop quit during setup"),
            IterateResult::Err(error) => {
                anyhow::bail!("PulseAudio mainloop failed during setup: {error}")
            }
        }
        if ready()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out while connecting PulseAudio capture");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::{
        append_recent_pcm_bytes, append_recent_silence, spectrum_from_interleaved_pcm,
        AUDIO_SPECTRUM_BINS,
    };

    fn stereo_sine(frequency_hz: f32, sample_rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|index| {
                let sample = (TAU * frequency_hz * index as f32 / sample_rate as f32).sin();
                [sample, sample]
            })
            .collect()
    }

    #[test]
    fn silence_produces_zero_spectrum_in_renderer_channel_order() {
        let spectrum = spectrum_from_interleaved_pcm(&vec![0.0; 2048 * 2], 48_000);
        assert!(spectrum.left.iter().all(|value| *value == 0.0));
        assert!(spectrum.right.iter().all(|value| *value == 0.0));

        let flattened = spectrum.flattened();
        assert_eq!(flattened.len(), AUDIO_SPECTRUM_BINS * 2);
        assert_eq!(&flattened[..AUDIO_SPECTRUM_BINS], spectrum.left.as_slice());
        assert_eq!(&flattened[AUDIO_SPECTRUM_BINS..], spectrum.right.as_slice());
    }

    #[test]
    fn sine_energy_is_concentrated_near_its_expected_mel_band() {
        let sample_rate = 48_000;
        let spectrum =
            spectrum_from_interleaved_pcm(&stereo_sine(1_500.0, sample_rate, 2048), sample_rate);
        let peak = spectrum
            .left
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .expect("spectrum has bins");

        // In the native 20 Hz..20 kHz Mel layout, bands 20..23 span roughly
        // 1.27..1.74 kHz. These bounds are independent of the analyzer code.
        assert!((20..=23).contains(&peak), "unexpected 1.5 kHz peak band {peak}");
        assert!(spectrum.left[peak] > 0.5);
        assert!((spectrum.left[peak] - spectrum.right[peak]).abs() < 0.001);
    }

    #[test]
    fn quiet_off_bin_bass_and_voice_have_visible_separate_stereo_responses() {
        for frames in [800, 1600, 4096] {
            let pcm = (0..frames)
                .flat_map(|index| {
                    let time = index as f32 / 48_000.0;
                    [0.02 * (TAU * 110.0 * time).sin(), 0.02 * (TAU * 440.0 * time).sin()]
                })
                .collect::<Vec<_>>();
            let spectrum = spectrum_from_interleaved_pcm(&pcm, 48_000);
            // Bass is in the first few Mel bands; 440 Hz is near band 8.
            let bass = spectrum.left[1..=3].iter().copied().fold(0.0_f32, f32::max);
            let voice = spectrum.right[7..=10].iter().copied().fold(0.0_f32, f32::max);
            assert!(bass > 0.45, "{frames} frames: quiet bass response {bass}");
            assert!(voice > 0.45, "{frames} frames: quiet voice response {voice}");
            assert!(spectrum.right[1..=3].iter().all(|value| *value < bass * 0.5));
            assert!(spectrum.left[7..=10].iter().all(|value| *value < voice * 0.5));
        }
    }

    #[test]
    fn quiet_broadband_audio_lights_up_most_bands_without_channel_crosstalk() {
        let mut random = 0x1234_5678_u32;
        let pcm = (0..4096)
            .flat_map(|_| {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (random as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32;
                [noise * 0.02, 0.0]
            })
            .collect::<Vec<_>>();
        let spectrum = spectrum_from_interleaved_pcm(&pcm, 48_000);
        let visible = spectrum.left.iter().filter(|value| **value > 0.15).count();
        assert!(visible >= 50, "only {visible} broadband bands are visible");
        assert!(spectrum.right.iter().all(|value| *value == 0.0));
        assert!(spectrum.left.iter().all(|value| value.is_finite() && *value <= 1.0));
    }

    #[test]
    fn visual_response_preserves_quiet_to_loud_dynamics_on_a_music_scale() {
        let responses = [0.002, 0.02, 0.2].map(|amplitude| {
            let pcm = stereo_sine(440.0, 48_000, 4096)
                .into_iter()
                .map(|value| value * amplitude)
                .collect::<Vec<_>>();
            spectrum_from_interleaved_pcm(&pcm, 48_000).left.iter().copied().fold(0.0_f32, f32::max)
        });
        // A 2% sine is about -34 dBFS: comfortably visible within the native
        // -80..-8 dB display range, with room for quieter and louder signals.
        assert!((0.55..0.85).contains(&responses[1]), "responses: {responses:?}");
        assert!(responses[0] > 0.15 && responses[0] + 0.1 < responses[1]);
        assert!(responses[1] + 0.1 < responses[2] && responses[2] <= 1.0);
    }

    #[test]
    fn boosted_float_pcm_does_not_introduce_harmonic_bands() {
        let reference = stereo_sine(440.0, 48_000, 4096)
            .into_iter()
            .map(|value| value * 0.25)
            .collect::<Vec<_>>();
        let baseline = spectrum_from_interleaved_pcm(&reference, 48_000);
        for gain in [6.0, 13.5] {
            // Valid float capture peaks of 1.5 and 3.375 must retain the sine's
            // shape. Clipping at unity would add third/fifth/seventh harmonics.
            let boosted = reference.iter().map(|value| value * gain).collect::<Vec<_>>();
            let spectrum = spectrum_from_interleaved_pcm(&boosted, 48_000);
            assert!(spectrum.left[7..=10].iter().any(|value| *value > 0.9));
            // Hand-selected Mel ranges around 1320, 2200 and 3080 Hz.
            for bands in [17..=21, 25..=29, 31..=35] {
                for band in bands {
                    assert!(
                        spectrum.left[band] <= baseline.left[band] + 0.02,
                        "gain {gain}: new harmonic response {} in band {band} (reference {})",
                        spectrum.left[band],
                        baseline.left[band]
                    );
                }
            }
        }
    }

    #[test]
    fn spectrum_ignores_old_backlog_and_unpaired_trailing_sample() {
        let recent = stereo_sine(110.0, 48_000, 4096);
        let mut backlog = stereo_sine(6107.0, 48_000, 8192);
        backlog.extend_from_slice(&recent);
        backlog.push(1.0);
        assert_eq!(
            spectrum_from_interleaved_pcm(&backlog, 48_000),
            spectrum_from_interleaved_pcm(&recent, 48_000)
        );
    }

    #[test]
    fn malformed_pcm_and_extreme_sample_rates_remain_finite_and_bounded() {
        let invalid = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::NAN].repeat(2048);
        let spectrum = spectrum_from_interleaved_pcm(&invalid, 48_000);
        assert!(spectrum.flattened().iter().all(|value| *value == 0.0));
        for sample_rate in [0, 1, 8000, 44_100, 48_000, 192_000, u32::MAX] {
            for frames in [0, 1, 2, 127, 4096] {
                let pcm = [f32::MAX, -f32::MAX].repeat(frames);
                assert!(spectrum_from_interleaved_pcm(&pcm, sample_rate)
                    .flattened()
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
            }
        }
    }

    #[test]
    fn pulse_backlog_retains_only_the_latest_bounded_sample_window() {
        let mut retained = vec![1.0_f32, 2.0];
        let incoming =
            (3_u32..=12).flat_map(|value| (value as f32).to_ne_bytes()).collect::<Vec<_>>();

        append_recent_pcm_bytes(&mut retained, &incoming, 4);
        assert_eq!(retained, vec![9.0, 10.0, 11.0, 12.0]);

        append_recent_silence(&mut retained, 10_000, 4);
        assert_eq!(retained, vec![0.0; 4]);
    }
}
