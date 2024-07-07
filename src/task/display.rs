use defmt::info;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_rp::i2c::Async;
use embassy_rp::i2c::{I2c};
use embassy_rp::peripherals::I2C0;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    image::Image,
    mono_font::{ascii::FONT_9X18_BOLD, MonoTextStyleBuilder},
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
    text::{Baseline, Text},
};
use ssd1306_async::{prelude::*, I2CDisplayInterface, Ssd1306};
use tinybmp::Bmp;

#[embassy_executor::task]
pub async fn display(
    spawner: Spawner,
    i2c_dsp_bus: &'static Mutex<NoopRawMutex, I2c<'_, I2C0, Async>>,
) {
    info!("Display task started");

    let interface = I2CDisplayInterface::new(I2cDevice::new(i2c_dsp_bus));
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().await.unwrap();

    let saber: Bmp<Gray8> = Bmp::from_slice(include_bytes!("../../resources/saber.bmp"))
        .expect("Failed to load BMP image");

    let im: Image<Bmp<Gray8>> = Image::new(&saber, Point::new(0, 0));
    im.draw(&mut display.color_converted()).unwrap();
    display.flush().await.unwrap();

    loop {
        Timer::after(Duration::from_millis(1_000)).await;
        info!("Tick");
        display.clear();
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_9X18_BOLD)
            .text_color(BinaryColor::On)
            .build();
        Text::with_baseline("Text 1", Point::zero(), text_style, Baseline::Top)
            .draw(&mut display)
            .unwrap();
        Text::with_baseline("Text 2", Point::new(0, 16), text_style, Baseline::Top)
            .draw(&mut display)
            .unwrap();

        display.flush().await.unwrap();

        Timer::after(Duration::from_millis(1_000)).await;
        info!("Tick");
        display.clear();
        im.draw(&mut display.color_converted()).unwrap();
        display.flush().await.unwrap();
    }
}
