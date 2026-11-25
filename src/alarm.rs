use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};

struct Beep {
    phase: f32,
    remaining: usize,
    sample_rate: f32,
    frequency: f32,
}

impl AudioCallback for Beep {
    type Channel = f32;

    fn callback(&mut self, output: &mut [f32]) {
        for sample in output {
            if self.remaining == 0 {
                *sample = 0.0;
                continue;
            }
            *sample = if (self.phase * self.frequency * std::f32::consts::TAU).sin() >= 0.0 {
                0.18
            } else {
                -0.18
            };
            self.phase += 1.0 / self.sample_rate;
            self.remaining -= 1;
        }
    }
}

pub struct AlarmSound {
    device: AudioDevice<Beep>,
}

impl AlarmSound {
    pub fn new(audio: &sdl2::AudioSubsystem) -> Result<Self, String> {
        let desired = AudioSpecDesired {
            freq: Some(44_100),
            channels: Some(1),
            samples: Some(1024),
        };
        let device = audio.open_playback(None, &desired, |spec| Beep {
            phase: 0.0,
            remaining: 0,
            sample_rate: spec.freq as f32,
            frequency: 880.0,
        })?;
        Ok(Self { device })
    }

    pub fn beep(&mut self, uncertain: bool) {
        {
            let mut callback = self.device.lock();
            callback.remaining = (callback.sample_rate * 0.22) as usize;
            callback.phase = 0.0;
            callback.frequency = if uncertain { 520.0 } else { 880.0 };
        }
        self.device.resume();
    }
}
