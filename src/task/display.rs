//! # Display task
//! This module contains the task that displays information on the OLED display.
//!
//! The task is responsible for initializing the display, displaying images and text, and updating the display.
use crate::task::{
    resources::{DisplayResources, Irqs},
    state::{BatteryLevel, OperationMode, DISPLAY_SIGNAL, STATE_MANAGER_MUTEX},
};
use crate::utility::string_utils::StringUtils;
use core::cell::RefCell;
use core::fmt::Write;
use defmt::{error, info, Debug2Format};
use embassy_executor::Spawner;
use embassy_rp::i2c::{Config, I2c};
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::{
    image::Image,
    mono_font::{
        ascii::{FONT_6X13, FONT_8X13_BOLD},
        MonoTextStyleBuilder,
    },
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
    text::{Baseline, Text},
};
use heapless::String;
use ssd1306_async::{prelude::*, I2CDisplayInterface, Ssd1306};
use tinybmp::Bmp;

/// Loads and holds BMP images and Points for the display
/// Holds some settings for composing the display
struct Settings<'a> {
    /// BMP image of the saber icon
    saber: Bmp<'static, Gray8>,
    /// BMP image of the colon icon
    colon: Bmp<'static, Gray8>,
    /// BMP images of the digits 0-9
    digits: [Bmp<'static, Gray8>; 10],
    /// BMP images of the battery status icons
    bat: [Bmp<'static, Gray8>; 6],
    /// BMP image of the battery mains icon
    bat_mains: Bmp<'static, Gray8>,
    /// BMP image of the settings icon
    settings: Bmp<'static, Gray8>,
    /// Position of the state indicator images, hight is 16
    state_indicator_position: Point,
    /// Position of the battery status images, hight is 11
    bat_position: Point,
    /// (Starting) Position of the time digits, hight is 24
    time_digit_start_position: Point,
    /// Position of the date text
    date_position: Point,
    /// (Starting) Position of content
    content_start_position: Point,
    /// Style of the state area text
    state_indicator_text_style: MonoTextStyle<'a, BinaryColor>,
    /// Style of the date text
    date_text_style: MonoTextStyle<'a, BinaryColor>,
    /// Style of the menu and system info content text
    content_text_style: MonoTextStyle<'a, BinaryColor>,
}

impl<'a> Settings<'a> {
    fn new() -> Self {
        Self {
            saber: Bmp::from_slice(include_bytes!("../../media/saber.bmp"))
                .expect("Failed to load BMP image"),
            colon: Bmp::from_slice(include_bytes!("../../media/colon.bmp"))
                .expect("Failed to load BMP image"),
            digits: [
                Bmp::from_slice(include_bytes!("../../media/0.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/1.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/2.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/3.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/4.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/5.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/6.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/7.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/8.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/9.bmp"))
                    .expect("Failed to load BMP image"),
            ],
            bat: [
                Bmp::from_slice(include_bytes!("../../media/bat_000.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/bat_020.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/bat_040.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/bat_060.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/bat_080.bmp"))
                    .expect("Failed to load BMP image"),
                Bmp::from_slice(include_bytes!("../../media/bat_100.bmp"))
                    .expect("Failed to load BMP image"),
            ],
            bat_mains: Bmp::from_slice(include_bytes!("../../media/bat_mains.bmp"))
                .expect("Failed to load BMP image"),
            settings: Bmp::from_slice(include_bytes!("../../media/settings.bmp"))
                .expect("Failed to load BMP image"),
            state_indicator_position: Point::new(0, 0),
            bat_position: Point::new(108, 0),
            time_digit_start_position: Point::new(13, 21),
            content_start_position: Point::new(0, 19),
            date_position: Point::new(0, 51),
            state_indicator_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_8X13_BOLD)
                .text_color(BinaryColor::On)
                .build(),
            date_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_6X13)
                .text_color(BinaryColor::On)
                .build(),
            content_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_6X13)
                .text_color(BinaryColor::On)
                .build(),
        }
    }
}

#[embassy_executor::task]
pub async fn display(
    _spawner: Spawner,
    r: DisplayResources,
    rtc_ref: &'static RefCell<Rtc<'static, RTC>>,
) {
    info!("Display task started");

    let scl = r.scl;
    let sda = r.sda;
    let mut config = Config::default();
    config.frequency = 400_000;
    let i2c = I2c::new_async(r.i2c0, scl, sda, Irqs, config);

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    match display.init().await {
        Ok(_) => {}
        Err(e) => {
            error!("Failed to initialize display: {}", defmt::Debug2Format(&e));
            return;
        }
    }

    display.set_brightness(Brightness::DIM).await.unwrap();

    let settings = Settings::new();

    loop {
        // Wait for a signal to update the display
        DISPLAY_SIGNAL.wait().await;

        // get the state of the system out of the mutex and quickly drop the mutex
        let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
        //let state_manager = state_manager_guard.as_ref().unwrap();
        let state_manager = match state_manager_guard.clone() {
            Some(state_manager) => state_manager,
            None => {
                error!("State manager not initialized");
                continue;
            }
        };
        drop(state_manager_guard);

        // get the system datetime
        let dt = match rtc_ref.borrow().now() {
            Ok(dt) => dt,
            Err(e) => {
                info!("RTC not running: {:?}", Debug2Format(&e));
                // return an empty DateTime
                DateTime {
                    year: 0,
                    month: 0,
                    day: 0,
                    day_of_week: DayOfWeek::Monday,
                    hour: 0,
                    minute: 0,
                    second: 0,
                }
            }
        };

        // prepare the display, note that nothing is sent to the display before flush()
        display.clear();

        '_state_area: {
            let state_indicator_position = settings.state_indicator_position.clone();
            match state_manager.operation_mode {
                OperationMode::Normal => match state_manager.alarm_settings.enabled {
                    true => {
                        let saber = Image::new(&settings.saber, state_indicator_position);
                        saber.draw(&mut display.color_converted()).unwrap();
                    }
                    false => {}
                },
                OperationMode::SetAlarmTime => {
                    let settings = Image::new(&settings.settings, state_indicator_position);
                    settings.draw(&mut display.color_converted()).unwrap();
                }
                OperationMode::Menu => {
                    Text::with_baseline(
                        "Menu",
                        settings.state_indicator_position,
                        settings.state_indicator_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
                OperationMode::SystemInfo => {
                    Text::with_baseline(
                        "Sys.-Info",
                        settings.state_indicator_position,
                        settings.state_indicator_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
                OperationMode::Alarm => {}
            }
        }

        '_battery_area: {
            let bat_image: Image<Bmp<'static, Gray8>>;
            let bat_position = settings.bat_position.clone();
            match state_manager.power_state.battery_level {
                BatteryLevel::Bat000 => {
                    bat_image = Image::new(&settings.bat[0], bat_position);
                }
                BatteryLevel::Bat020 => {
                    bat_image = Image::new(&settings.bat[1], bat_position);
                }
                BatteryLevel::Bat040 => {
                    bat_image = Image::new(&settings.bat[2], bat_position);
                }
                BatteryLevel::Bat060 => {
                    bat_image = Image::new(&settings.bat[3], bat_position);
                }
                BatteryLevel::Bat080 => {
                    bat_image = Image::new(&settings.bat[4], bat_position);
                }
                BatteryLevel::Bat100 => {
                    bat_image = Image::new(&settings.bat[5], bat_position);
                }
                BatteryLevel::Charging => {
                    bat_image = Image::new(&settings.bat_mains, bat_position);
                }
            }
            bat_image.draw(&mut display.color_converted()).unwrap();
        };

        '_main_area: {
            let hours: u8;
            let minutes: u8;
            match state_manager.operation_mode {
                OperationMode::Normal | OperationMode::Alarm => {
                    hours = dt.hour;
                    minutes = dt.minute;
                }
                OperationMode::SetAlarmTime => {
                    hours = state_manager.alarm_settings.time.0;
                    minutes = state_manager.alarm_settings.time.1;
                }
                _ => {
                    hours = 0;
                    minutes = 0;
                }
            };
            match state_manager.operation_mode {
                OperationMode::Normal | OperationMode::Alarm | OperationMode::SetAlarmTime => {
                    // Display the time
                    let mut digit_next_position = settings.time_digit_start_position.clone();
                    let first_hour_digit_index = (hours / 10) as usize;
                    let first_hour_digit = Image::new(
                        &settings.digits[first_hour_digit_index],
                        digit_next_position,
                    );

                    digit_next_position.x += 24;
                    let second_hour_digit_index = (hours % 10) as usize;
                    let second_hour_digit = Image::new(
                        &settings.digits[second_hour_digit_index],
                        digit_next_position,
                    );

                    digit_next_position.x += 24;
                    let colon = Image::new(&settings.colon, digit_next_position);

                    digit_next_position.x += 11;
                    let first_minute_digit_index = (minutes / 10) as usize;
                    let first_minute_digit = Image::new(
                        &settings.digits[first_minute_digit_index],
                        digit_next_position,
                    );

                    digit_next_position.x += 24;
                    let second_minute_digit_index = (minutes % 10) as usize;
                    let second_minute_digit = Image::new(
                        &settings.digits[second_minute_digit_index],
                        digit_next_position,
                    );

                    first_hour_digit
                        .draw(&mut display.color_converted())
                        .unwrap();
                    second_hour_digit
                        .draw(&mut display.color_converted())
                        .unwrap();
                    colon.draw(&mut display.color_converted()).unwrap();
                    first_minute_digit
                        .draw(&mut display.color_converted())
                        .unwrap();
                    second_minute_digit
                        .draw(&mut display.color_converted())
                        .unwrap();
                }
                OperationMode::Menu => {
                    let mut content_next_position = settings.content_start_position.clone();
                    Text::with_baseline(
                        "Green: Sys. Info",
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                    content_next_position.y += 15;
                    Text::with_baseline(
                        "Blue: Standby",
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                    content_next_position.y += 15;
                    Text::with_baseline(
                        "Yellow: Back",
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
                OperationMode::SystemInfo => {
                    let mut content_next_position = settings.content_start_position.clone();
                    let vsys = state_manager.power_state.vsys.clone();
                    let usb_power = state_manager.power_state.usb_power.clone();
                    let upper = state_manager
                        .power_state
                        .battery_voltage_fully_charged
                        .clone();
                    let lower = state_manager.power_state.battery_voltage_empty.clone();
                    let mut vsys_txt: String<20> = String::new();
                    let _ = write!(vsys_txt, "Vsys:  {}V", vsys);
                    Text::with_baseline(
                        &vsys_txt,
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                    content_next_position.y += 15;
                    let mut usb_txt: String<20> = String::new();
                    let _ = write!(usb_txt, "USB:   {}", usb_power);
                    Text::with_baseline(
                        &usb_txt,
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                    content_next_position.y += 15;
                    let mut bounds_txt: String<20> = String::new();
                    let _ = write!(bounds_txt, "Upper/Lower {}/{}V", upper, lower);
                    Text::with_baseline(
                        &bounds_txt,
                        content_next_position,
                        settings.content_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
            };
        }

        '_date_area: {
            match state_manager.operation_mode {
                OperationMode::Normal | OperationMode::Alarm => {
                    let date = StringUtils::convert_datetime_to_str(dt);
                    Text::with_baseline(
                        &date,
                        settings.date_position,
                        settings.date_text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
                _ => {}
            }
        };

        // finally: send the display buffer to the display
        display.flush().await.unwrap();
        // and we are done for this cycle
    }
}
