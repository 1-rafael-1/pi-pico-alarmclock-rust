use crate::task::resources::{DisplayResources, Irqs};
use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::i2c::{Config, I2c};
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
pub async fn display(_spawner: Spawner, r: DisplayResources) {
    info!("Display task started");

    let scl = r.scl;
    let sda = r.sda;
    let mut config = Config::default();
    config.frequency = 400_000;
    let i2c = I2c::new_async(r.i2c0, scl, sda, Irqs, config);

    let interface = I2CDisplayInterface::new(i2c);
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

        display.clear();
        im.draw(&mut display.color_converted()).unwrap();
        display.flush().await.unwrap();
    }
}
