use core::fmt;
use std::collections::VecDeque;

use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Media::Audio::eConsole;
use windows::Win32::Media::Audio::eRender;
use windows::Win32::Media::Audio::IAudioClient;
use windows::Win32::Media::Audio::IAudioRenderClient;
use windows::Win32::Media::Audio::IMMDevice;
use windows::Win32::Media::Audio::IMMDeviceEnumerator;
use windows::Win32::Media::Audio::MMDeviceEnumerator;
use windows::Win32::Media::Audio::PKEY_AudioEngine_DeviceFormat;
use windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT;
use windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_EXCLUSIVE;
use windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_SHARED;
use windows::Win32::Media::Audio::AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
use windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE;
use windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE;
use windows::Win32::System::Com::CoCreateInstance;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Com::STGM_READ;
use windows::Win32::System::Threading::AvRevertMmThreadCharacteristics;
use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::System::Threading::INFINITE;

use super::AudioDriver;

pub enum Error {
    DeviceNotFound(String),
    FromUtf16Error(std::string::FromUtf16Error),
    WaitTimeout,
    WindowsError(windows::core::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::DeviceNotFound(name) => write!(f, "Device not found: {}", name),
            Error::FromUtf16Error(error) => write!(f, "FromUtf16Error: {}", error),
            Error::WaitTimeout => write!(f, "WaitTimeout"),
            Error::WindowsError(error) => write!(f, "WindowsError: {}", error),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::DeviceNotFound(name) => write!(f, "DeviceNotFound({})", name),
            Error::FromUtf16Error(error) => write!(f, "FromUtf16Error({:?})", error),
            Error::WaitTimeout => write!(f, "WaitTimeout"),
            Error::WindowsError(error) => write!(f, "WindowsError({:?})", error),
        }
    }
}

impl From<std::string::FromUtf16Error> for Error {
    fn from(error: std::string::FromUtf16Error) -> Self {
        Error::FromUtf16Error(error)
    }
}

impl From<windows::core::Error> for Error {
    fn from(error: windows::core::Error) -> Self {
        Error::WindowsError(error)
    }
}

struct WASAPIDriverPrev {
    audio_client: IAudioClient,
    _audio_device: IMMDevice,
    buffer_size: u32,
    channels: u16,
    _device_period: i64,
    event_handle: HANDLE,
    exclusive: bool,
    frequency: u32,
    latency: i64,
    mode: u32,
    precision: u16,
    render_client: IAudioRenderClient,
    samples: VecDeque<Vec<f64>>,
    task_handle: Option<HANDLE>,
}

impl WASAPIDriverPrev {
    fn new(
        audio_device: IMMDevice,
        exclusive: bool,
        latency: i64,
    ) -> Result<WASAPIDriverPrev, Error> {
        let audio_client = unsafe { audio_device.Activate::<IAudioClient>(CLSCTX_ALL, None)? };

        let mut device_period = 0i64;
        let mut task_handle = None;
        let wave_format: WAVEFORMATEXTENSIBLE;

        if exclusive {
            let property_store = unsafe { audio_device.OpenPropertyStore(STGM_READ) }?;
            let property_variant =
                unsafe { property_store.GetValue(&PKEY_AudioEngine_DeviceFormat) }?;
            wave_format = unsafe {
                property_variant
                    .Anonymous
                    .Anonymous
                    .Anonymous
                    .blob
                    .pBlobData
                    .cast::<WAVEFORMATEXTENSIBLE>()
                    .as_ref()
                    .unwrap()
                    .clone()
            };

            let mut device_period = 0i64;
            unsafe { audio_client.GetDevicePeriod(None, Some(&mut device_period))? };

            let latency = device_period.max(latency * 10_000); // 1ms -> 100 ns units
            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    latency,
                    latency,
                    &wave_format.Format as *const _,
                    None,
                )
            }?;

            let mut task_index = 0u32;
            task_handle =
                Some(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }?);
        } else {
            let wave_format_ex = unsafe { audio_client.GetMixFormat()? };
            wave_format = unsafe { wave_format_ex.cast::<WAVEFORMATEXTENSIBLE>().as_ref() }
                .unwrap()
                .clone();
            unsafe { CoTaskMemFree(Some(wave_format_ex as *const _)) };

            unsafe { audio_client.GetDevicePeriod(None, Some(&mut device_period))? };

            let latency = device_period.max(latency * 10_000); // 1ms -> 100 ns units
            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    latency,
                    0,
                    &wave_format.Format as *const _,
                    None,
                )
            }?;
        };

        let event_handle = unsafe { CreateEventW(None, false, false, None) }?;
        unsafe { audio_client.SetEventHandle(event_handle) }?;

        let render_client = unsafe { audio_client.GetService::<IAudioRenderClient>()? };
        let buffer_size = unsafe { audio_client.GetBufferSize()? };

        let samples = VecDeque::with_capacity(buffer_size as usize);

        unsafe { audio_client.Reset()? };
        unsafe { audio_client.Start()? };

        Ok(WASAPIDriverPrev {
            audio_client,
            _audio_device: audio_device,
            buffer_size,
            channels: wave_format.Format.nChannels,
            _device_period: device_period,
            event_handle,
            exclusive,
            frequency: wave_format.Format.nSamplesPerSec,
            latency,
            mode: wave_format.SubFormat.data1,
            precision: wave_format.Format.wBitsPerSample,
            render_client,
            samples,
            task_handle,
        })
    }

    fn write(&mut self) -> Result<(), Error> {
        let available = if !self.exclusive {
            let padding = unsafe { self.audio_client.GetCurrentPadding()? };
            self.buffer_size - padding
        } else {
            self.buffer_size
        };
        let length = available.min(self.samples.len() as u32);

        let mut buffer = unsafe { self.render_client.GetBuffer(length) }?;
        let mut buffer_flags = 0;
        for _ in 0..length as usize {
            let sample = self.samples.pop_front().unwrap();

            if self.mode == 1 && self.precision == 16 {
                let output = unsafe {
                    std::slice::from_raw_parts_mut(buffer as *mut i16, self.channels as usize)
                };
                for (output, sample) in output.iter_mut().zip(sample.iter()) {
                    *output = (sample * (32768.0 - 1.0)) as i16; // 2^15 - 1, i16::max as f64
                }
                buffer = unsafe { buffer.offset(self.channels as isize * 2) };
            } else if self.mode == 1 && self.precision == 32 {
                let output = unsafe {
                    std::slice::from_raw_parts_mut(buffer as *mut i32, self.channels as usize)
                };
                for (output, sample) in output.iter_mut().zip(sample.iter()) {
                    *output = (sample * (2147483648.0 - 1.0)) as i32; // 2^31 - 1, i32::max as f64
                }
                buffer = unsafe { buffer.offset(self.channels as isize * 4) };
            } else if self.mode == 3 && self.precision == 32 {
                let output = unsafe {
                    std::slice::from_raw_parts_mut(buffer as *mut f32, self.channels as usize)
                };
                for (output, sample) in output.iter_mut().zip(sample.iter()) {
                    *output = sample.min(1.0).max(-1.0) as f32;
                }
                buffer = unsafe { buffer.offset(self.channels as isize * 4) };
            } else {
                //output silence for unsupported sample formats
                buffer_flags = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
                break;
            }
        }
        unsafe { self.render_client.ReleaseBuffer(length, buffer_flags) }?;

        Ok(())
    }

    fn clear(&mut self) -> Result<(), Error> {
        self.samples.clear();
        unsafe {
            self.audio_client.Stop()?;
            self.audio_client.Reset()?;
            self.audio_client.Start()?;
        }
        Ok(())
    }
}

impl Drop for WASAPIDriverPrev {
    fn drop(&mut self) {
        if let Err(err) = unsafe { self.audio_client.Stop() } {
            eprintln!("IAudioClient::Stop failed: {:?}", err);
        }

        if let Err(err) = unsafe { CloseHandle(self.event_handle) } {
            eprintln!("CloseHandle failed: {:?}", err);
        }

        if let Some(task_handle) = self.task_handle {
            match unsafe { AvRevertMmThreadCharacteristics(task_handle) } {
                Err(err) => eprintln!("AvRevertMmThreadCharacteristics failed: {:?}", err),
                _ => {}
            };
        }
    }
}

pub struct WASAPIDriver {
    prev: WASAPIDriverPrev,
    current_device_name: String,
    device_names: Vec<String>,
    device_ids: Vec<String>,
    enumlator: IMMDeviceEnumerator,
    blocking: bool,
}

fn str_to_pcwstr(s: &str) -> Vec<u16> {
    let result = s
        .to_string()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    result
}

impl WASAPIDriver {
    pub fn driver() -> &'static str {
        "WASAPI"
    }

    pub fn new() -> Result<Self, Error> {
        let enumlator = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)?
        };

        let audio_device = unsafe { enumlator.GetDefaultAudioEndpoint(eRender, eConsole)? };

        let default_device_id = unsafe { audio_device.GetId()?.to_string()? };

        let device_collection =
            unsafe { enumlator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { device_collection.GetCount()? };

        let mut device_names = Vec::with_capacity(count as usize);
        let mut device_ids = Vec::with_capacity(count as usize);

        for i in 0..count {
            let device_context = unsafe { device_collection.Item(i) }?;
            let id = unsafe { device_context.GetId()?.to_string()? };
            let property_store = unsafe { device_context.OpenPropertyStore(STGM_READ) }?;
            let property_variant = unsafe { property_store.GetValue(&PKEY_Device_FriendlyName) }?;
            let name = unsafe {
                property_variant
                    .Anonymous
                    .Anonymous
                    .Anonymous
                    .pwszVal
                    .to_string()
            }?;

            if id == default_device_id {
                device_ids.insert(0, id);
                device_names.insert(0, name);
            } else {
                device_ids.push(id);
                device_names.push(name);
            }
        }

        let prev = WASAPIDriverPrev::new(audio_device, false, 40)?;

        Ok(WASAPIDriver {
            prev,
            current_device_name: device_names[0].clone(),
            device_names,
            device_ids,
            enumlator,
            blocking: true,
        })
    }

    pub fn reset(&mut self) -> Result<(), Error> {
        let device_id: &str = &self
            .device_ids
            .iter()
            .zip(self.device_names.iter())
            .find_map(|(id, name)| {
                if **name == self.current_device_name {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or(Error::DeviceNotFound(self.current_device_name.clone()))?;

        let device = unsafe {
            self.enumlator
                .GetDevice(PCWSTR::from_raw(str_to_pcwstr(device_id).as_ptr()))?
        };

        self.prev = WASAPIDriverPrev::new(device, self.prev.exclusive, self.prev.latency)?;

        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), Error> {
        self.prev.clear()
    }
}

impl AudioDriver for WASAPIDriver {
    fn driver(&self) -> &'static str {
        "WASAPI"
    }

    fn support_exclusive(&self) -> bool {
        true
    }

    fn support_device_list(&self) -> Vec<String> {
        self.device_names.clone()
    }

    fn support_blocking(&self) -> bool {
        true
    }

    fn support_channels(&self) -> Vec<u32> {
        vec![self.prev.channels as u32]
    }

    fn support_frequencies(&self) -> Vec<u32> {
        vec![self.prev.frequency]
    }

    fn support_latencies(&self) -> Vec<u32> {
        vec![0, 20, 40, 60, 80, 100]
    }

    fn set_exclusive(&mut self, exclusive: bool) -> Result<(), super::Error> {
        if self.prev.exclusive == exclusive {
            return Ok(());
        }

        self.prev.exclusive = exclusive;
        self.reset()?;
        Ok(())
    }

    fn set_device(&mut self, device: &str) -> Result<(), super::Error> {
        if self.current_device_name == device {
            return Ok(());
        }

        self.current_device_name = device.to_owned();
        self.reset()?;
        Ok(())
    }

    fn set_blocking(&mut self, blocking: bool) -> Result<(), super::Error> {
        if self.blocking == blocking {
            return Ok(());
        }

        self.blocking = blocking;
        Ok(())
    }

    fn set_latency(&mut self, latency: u32) -> Result<(), super::Error> {
        if self.prev.latency == latency as i64 {
            return Ok(());
        }

        self.prev.latency = latency as i64;
        self.reset()?;
        Ok(())
    }

    fn output(&mut self, samples: &[f64]) -> Result<(), super::Error> {
        let samples = samples[0..self.prev.channels as usize].to_vec();
        self.prev.samples.push_back(samples);

        if self.prev.samples.len() >= self.prev.buffer_size as usize {
            if unsafe {
                WaitForSingleObject(
                    self.prev.event_handle,
                    if self.blocking { INFINITE } else { 0 },
                )
            } == WAIT_OBJECT_0
            {
                self.prev.write()?;
            } else {
                return Err(super::Error::WASAPIError(Error::WaitTimeout));
            }
        }

        Ok(())
    }

    fn output_i16(&mut self, samples: &[i16]) -> Result<(), super::Error> {
        let samples = samples[0..self.prev.channels as usize]
            .iter()
            .map(|&x| x as f64 / 32768.0)
            .collect();
        self.prev.samples.push_back(samples);

        if self.prev.samples.len() >= self.prev.buffer_size as usize {
            if unsafe {
                WaitForSingleObject(
                    self.prev.event_handle,
                    if self.blocking { INFINITE } else { 0 },
                )
            } == WAIT_OBJECT_0
            {
                self.prev.write()?;
            } else {
                return Err(super::Error::WASAPIError(Error::WaitTimeout));
            }
        }

        Ok(())
    }
}
