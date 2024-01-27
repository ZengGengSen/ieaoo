use windows::Win32::System::Com::{CoInitialize, CoUninitialize};

const SAMPLE_U8: &[u8] = include_bytes!("test.pcm");

fn main() {
    unsafe { CoInitialize(None).unwrap() };

    let samples = unsafe {
        std::slice::from_raw_parts(SAMPLE_U8.as_ptr() as *const i16, SAMPLE_U8.len() / 2)
    };

    // player
    let mut audio = ieaoo::audio::Audio::new(ieaoo::audio::AudioDriverType::WASAPI).unwrap();

    for sample in samples.chunks(2) {
        audio.output(&[sample[0] as f64 / 32768.0, sample[1] as f64 / 32768.0]).unwrap();
    }

    unsafe { CoUninitialize() };
}
