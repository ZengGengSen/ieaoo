use std::ffi::CString;

use alsa::{device_name::HintIter, PCM, Direction, pcm::{Access, Format, HwParams, Frames}, ValueOr};

use super::AudioDriver;

pub use alsa::Error;

struct ALSADriverPrev {
    blocking: bool,
    buffer: Vec<i16>,
    buffer_size: u64,
    frequency: u32,
    latency: u32,
    name: String,
    pcm: PCM,
    period_size: u64,
}

impl ALSADriverPrev {
    fn new(name: &str, latency: u32, frequency: u32, blocking: bool) -> Result<ALSADriverPrev, super::Error> {
        let pcm = PCM::new(&name, Direction::Playback, !blocking)?;

        let rate = frequency;
        let buffer_time = latency * 1000;  // ms -> us
        let period_time = buffer_time / 8; // ms -> us

        let hw_params = HwParams::any(&pcm)?;
        hw_params.set_access(Access::RWInterleaved)?;
        // todo: add support for other formats
        hw_params.set_format(Format::S16LE)?;
        // todo: add support for other channels
        hw_params.set_channels(2)?;
        hw_params.set_rate_near(rate, ValueOr::Nearest)?;
        hw_params.set_buffer_time_near(buffer_time, ValueOr::Nearest)?;
        hw_params.set_period_time_near(period_time, ValueOr::Nearest)?;
        pcm.hw_params(&hw_params)?;
        drop(hw_params);

        let (buffer_size, period_size) = pcm.get_params()?;

        let sw_params = pcm.sw_params_current()?;
        sw_params.set_start_threshold(buffer_size as Frames / 2)?;
        pcm.sw_params(&sw_params)?;
        drop(sw_params);

        Ok(ALSADriverPrev {
            blocking,
            buffer_size,
            frequency,
            latency,
            buffer: Vec::with_capacity(period_size as usize * 2),
            name: name.to_string(),
            pcm,
            period_size,
        })
    }

    fn write(&mut self) -> Result<(), super::Error> {
        loop {
            let available = match self.pcm.avail_update() {
                Ok(it) => it,
                Err(err) => {
                    self.pcm.recover(err.errno() as i32, true)?;
                    continue;
                }
            };

            if available < self.buffer.len() as Frames {
                if let Err(err) = self.pcm.wait(None) {
                    self.pcm.recover(err.errno() as i32, true)?;
                }
            }

            if available >= self.buffer.len() as Frames {
                break;
            }
        };

        let mut output = self.buffer.as_slice();

        let mut i = 4;
        while output.len() > 0 && i >= 0 {
            i -= 1;

            let io_i16 = self.pcm.io_i16()?;

            match io_i16.writei(output) {
                Ok(written) => {
                    if written * 2 <= output.len() {
                        output = &output[written as usize * 2..];
                    }
                },
                Err(err) => {
                    //no samples written
                    if let Err(err) = self.pcm.recover(err.errno() as i32, true) {
                        eprintln!("ALSA: {}", err);
                    }
                }
            }
        }

        if i < 0 {
            let (r, s, remain) = if output.len() == self.buffer.len() {
                (2.., 0, self.buffer.len() - 2)
            } else {
                (self.buffer.len() - output.len().., 0, output.len())
            };

            self.buffer.copy_within(r, s);
            self.buffer.truncate(remain);
        } else {
            let remain = output.len();
            self.buffer.truncate(remain);
        }

        Ok(())
    }
}

pub struct ALSADriver {
    device_names: Vec<String>,
    prev: ALSADriverPrev,
}

impl ALSADriver {
    pub fn new() -> Result<ALSADriver, super::Error> {
        let device_names = HintIter::new(None, &CString::new("pcm").unwrap())?
            .map(|hint| {
                hint.name.unwrap().clone()
            })
            .collect::<Vec<_>>();

        if device_names.len() == 0 {
            return Err(super::Error::NoDevice);
        }

        let prev = ALSADriverPrev::new(&device_names[0], 20, 44100, false)?;

        Ok(ALSADriver {
            device_names,
            prev,
        })
    }
}

impl AudioDriver for ALSADriver {
    fn driver(&self) -> &'static str {
        "ALSA"
    }

    fn support_device_list(&self) -> Vec<String> {
        self.device_names.clone()
    }

    fn support_blocking(&self) -> bool {
        true
    }

    fn support_channels(&self) -> Vec<u32> {
        vec![2]
    }

    fn support_frequencies(&self) -> Vec<u32> {
        vec![44100, 48000, 96000]
    }

    fn support_latencies(&self) -> Vec<u32> {
        vec![20, 40, 60, 80, 100]
    }

    fn set_device(&mut self, device: &str) -> Result<(), super::Error> {
        if !self.device_names.contains(&device.to_string()) {
            return Err(super::Error::DeviceNotFound(device.to_string()));
        }

        if self.prev.name == device.to_string() {
            return Ok(());
        }

        self.prev = ALSADriverPrev::new(device, self.prev.latency, self.prev.frequency, self.prev.blocking)?;
        Ok(())
    }

    fn set_blocking(&mut self, blocking: bool) -> Result<(), super::Error> {
        if self.prev.blocking == blocking {
            return Ok(());
        }

        self.prev = ALSADriverPrev::new(&self.prev.name, self.prev.latency, self.prev.frequency, blocking)?;
        Ok(())
    }

    fn set_frequency(&mut self, frequency: u32) -> Result<(), super::Error> {
        if !self.support_frequencies().contains(&frequency) {
            return Err(super::Error::Unsupported(format!("frequency: {}", frequency)));
        }

        if self.prev.frequency == frequency {
            return Ok(());
        }

        self.prev = ALSADriverPrev::new(&self.prev.name, self.prev.latency, frequency, self.prev.blocking)?;
        Ok(())
    }

    fn set_latency(&mut self, latency: u32) -> Result<(), super::Error> {
        if !self.support_latencies().contains(&latency) {
            return Err(super::Error::Unsupported(format!("latency: {}", latency)));
        }

        if self.prev.latency == latency {
            return Ok(());
        }

        self.prev = ALSADriverPrev::new(&self.prev.name, latency, self.prev.frequency, self.prev.blocking)?;
        Ok(())
    }

    fn output(&mut self, samples: &[f64]) -> Result<(), super::Error> {
        self.prev.buffer.push((samples[0] * 32767.0) as i16);
        self.prev.buffer.push((samples[1] * 32767.0) as i16);

        println!("{} {}", self.prev.buffer.len(), self.prev.period_size as usize * 2);
        if self.prev.buffer.len() >= self.prev.period_size as usize * 2 {
            self.prev.write()?;
        }

        Ok(())
    }
}
