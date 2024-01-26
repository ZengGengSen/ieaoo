#[cfg(target_os = "windows")]
mod wasapi;

#[cfg(target_os = "windows")]
pub use wasapi::WASAPIDriver;

pub enum AudioDriverType {
    #[cfg(target_os = "windows")]
    WASAPI,
    None,
}

pub enum Error {
    Unsupported(String),
    #[cfg(target_os = "windows")]
    WASAPIError(wasapi::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
       match self {
           Error::Unsupported(msg) => write!(f, "Unsupported: {}", msg),
           #[cfg(target_os = "windows")]
           Error::WASAPIError(err) => write!(f, "WASAPIError: {}", err),
       }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
       match self {
           Error::Unsupported(msg) => write!(f, "Unsupported: {}", msg),
           #[cfg(target_os = "windows")]
           Error::WASAPIError(err) => write!(f, "WASAPIError: {}", err),
       }
    }
}

#[cfg(target_os = "windows")]
impl From<wasapi::Error> for Error {
    fn from(err: wasapi::Error) -> Self {
        Error::WASAPIError(err)
    }
}

pub trait AudioDriver {
    fn driver(&self) -> &'static str {
        "None"
    }

    fn support_exclusive(&self) -> bool {
        false
    }

    fn support_device_list(&self) -> Vec<String> {
        Vec::new()
    }

    fn support_blocking(&self) -> bool {
        false
    }

    fn support_channels(&self) -> Vec<u32> {
        Vec::new()
    }

    fn support_frequencies(&self) -> Vec<u32> {
        Vec::new()
    }

    fn support_latencies(&self) -> Vec<u32> {
        Vec::new()
    }

    fn set_exclusive(&mut self, exclusive: bool) -> Result<(), Error> {
        let _ = exclusive;
        Ok(())
    }

    fn set_device(&mut self, device: &str) -> Result<(), Error> {
        let _ = device;
        Ok(())
    }

    fn set_blocking(&mut self, blocking: bool) -> Result<(), Error> {
        let _ = blocking;
        Ok(())
    }

    fn set_channels(&mut self, channels: u32) -> Result<(), Error> {
        let _ = channels;
        Ok(())
    }

    fn set_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        let _ = frequency;
        Ok(())
    }

    fn set_latency(&mut self, latency: u32) -> Result<(), Error> {
        let _ = latency;
        Ok(())
    }

    fn output(&mut self, samples: &[f64]) -> Result<(), Error> {
        let _ = samples;
        Ok(())
    }
}

pub struct NullDriver;

impl AudioDriver for NullDriver {}

pub struct Audio {
    instance: Box<dyn AudioDriver>,
}

impl Audio {
    pub fn new(ty: AudioDriverType) -> Result<Self, Error> {
        match ty {
            #[cfg(target_os = "windows")]
            AudioDriverType::WASAPI => Ok(Audio {
                instance: Box::new(WASAPIDriver::new()?),
            }),
            _ => Ok(Audio {
                instance: Box::new(NullDriver),
            }),
        }
    }

    pub fn support_drivers() -> Vec<&'static str> {
        let mut drivers = Vec::new();
        #[cfg(target_os = "windows")]
        drivers.push("WASAPI");
        drivers
    }

    pub fn support_exclusive(&self) -> bool {
        self.instance.support_exclusive()
    }

    pub fn support_device_list(&self) -> Vec<String> {
        self.instance.support_device_list()
    }

    pub fn support_blocking(&self) -> bool {
        self.instance.support_blocking()
    }

    pub fn support_channels(&self) -> Vec<u32> {
        self.instance.support_channels()
    }

    pub fn support_frequencies(&self) -> Vec<u32> {
        self.instance.support_frequencies()
    }

    pub fn support_latencies(&self) -> Vec<u32> {
        self.instance.support_latencies()
    }

    pub fn set_exclusive(&mut self, exclusive: bool) -> Result<(), Error> {
        if self.instance.support_exclusive() {
            self.instance.set_exclusive(exclusive)
        } else {
            Err(Error::Unsupported(
                "Exclusive mode is not supported".to_string(),
            ))
        }
    }

    pub fn set_device(&mut self, device: &str) -> Result<(), Error> {
        if self
            .instance
            .support_device_list()
            .contains(&device.to_string())
        {
            self.instance.set_device(device)
        } else {
            Err(Error::Unsupported(format!(
                "Device {} is not supported",
                device
            )))
        }
    }

    pub fn set_blocking(&mut self, blocking: bool) -> Result<(), Error> {
        if self.instance.support_blocking() {
            self.instance.set_blocking(blocking)
        } else {
            Err(Error::Unsupported(
                "Blocking mode is not supported".to_string(),
            ))
        }
    }

    pub fn set_channels(&mut self, channels: u32) -> Result<(), Error> {
        if self.instance.support_channels().contains(&channels) {
            self.instance.set_channels(channels)
        } else {
            Err(Error::Unsupported(format!(
                "Channels {} is not supported",
                channels
            )))
        }
    }

    pub fn set_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        if self.instance.support_frequencies().contains(&frequency) {
            self.instance.set_frequency(frequency)
        } else {
            Err(Error::Unsupported(format!(
                "Frequency {} is not supported",
                frequency
            )))
        }
    }

    pub fn set_latency(&mut self, latency: u32) -> Result<(), Error> {
        if self.instance.support_latencies().contains(&latency) {
            self.instance.set_latency(latency)
        } else {
            Err(Error::Unsupported(format!(
                "Latency {} is not supported",
                latency
            )))
        }
    }

    pub fn output(&mut self, sample: &[f64]) -> Result<(), Error> {
        self.instance.output(sample)?;
        Ok(())
    }
}
