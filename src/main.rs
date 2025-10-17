//! # Main
//! This is the main entry point of the program.
//! we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]
#![allow(clippy::unwrap_used)]

use crate::task::alarm_settings::alarm_settings_handler;
use crate::task::alarm_trigger::alarm_trigger_task;
use crate::task::buttons::{blue_button_handler, green_button_handler, yellow_button_handler};
use crate::task::display::display_handler;
use crate::task::orchestrate::{alarm_expirer, orchestrator, scheduler};
use crate::task::power::{usb_power_detector, vsys_voltage_reader};
use crate::task::sound::sound_handler;
use crate::task::time_updater::time_updater;
use defmt::info;
use embassy_executor::{Spawner, main};
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcInterruptHandler};
use embassy_rp::bind_interrupts;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Config as I2cConfig, I2c, InterruptHandler as I2cInterruptHandler};
use embassy_rp::peripherals::{I2C0, PIO0, UART1};
use embassy_rp::pio::InterruptHandler as PioInterruptHandler;
use embassy_rp::rtc::{InterruptHandler as RtcInterruptHandler, Rtc};
use embassy_rp::spi::{Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::uart::{BufferedInterruptHandler, BufferedUart, Config as UartConfig};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// import the task module (submodule of src)
mod task;

// import the utility module (submodule of src)
mod utility;

// Bind the interrupts on a global scope for convenience
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
    UART1_IRQ => BufferedInterruptHandler<UART1>;
    ADC_IRQ_FIFO => AdcInterruptHandler;
    RTC_IRQ => RtcInterruptHandler;
});

/// The main entry point of the program. This is where the tasks are spawned and run.
#[main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040
    let p = embassy_rp::init(Default::default());

    // Orchestrator tasks
    spawner.spawn(orchestrator().unwrap());
    spawner.spawn(scheduler().unwrap());
    spawner.spawn(alarm_expirer().unwrap());
    spawner.spawn(alarm_trigger_task().unwrap());

    // Green button
    let btn_green = Input::new(p.PIN_20, Pull::Up);
    spawner.spawn(green_button_handler(btn_green).unwrap());

    // Blue button
    let btn_blue = Input::new(p.PIN_21, Pull::Up);
    spawner.spawn(blue_button_handler(btn_blue).unwrap());

    // Yellow button
    let btn_yellow = Input::new(p.PIN_22, Pull::Up);
    spawner.spawn(yellow_button_handler(btn_yellow).unwrap());

    // USB power detector
    let vbus_in = Input::new(p.PIN_28, Pull::None);
    spawner.spawn(usb_power_detector(vbus_in).unwrap());

    // Vsys voltage reader
    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let vsys_channel = Channel::new_pin(p.PIN_27, Pull::None);
    spawner.spawn(vsys_voltage_reader(adc, vsys_channel).unwrap());

    // Display
    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = 400_000;
    let i2c = I2c::new_async(p.I2C0, p.PIN_13, p.PIN_12, Irqs, i2c_config);
    spawner.spawn(display_handler(i2c).unwrap());

    // DFPlayer sound handler
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 9600;
    static TX_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    static RX_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    let tx_buf = TX_BUFFER.init([0u8; 256]);
    let rx_buf = RX_BUFFER.init([0u8; 256]);
    let uart = BufferedUart::new(p.UART1, p.PIN_4, p.PIN_5, Irqs, tx_buf, rx_buf, uart_config);
    let dfplayer_pwr = Output::new(p.PIN_8, Level::Low);
    spawner.spawn(sound_handler(uart, dfplayer_pwr).unwrap());

    // Alarm settings persistence
    const FLASH_SIZE: usize = 2 * 1024 * 1024;
    let flash = Flash::<_, Async, FLASH_SIZE>::new(p.FLASH, p.DMA_CH4);
    spawner.spawn(alarm_settings_handler(flash).unwrap());

    // Time updater with WiFi and RTC
    let rtc = Rtc::new(p.RTC, Irqs);
    let wifi_peripherals = crate::task::time_updater::WifiPeripherals {
        pwr_pin: p.PIN_23,
        cs_pin: p.PIN_25,
        pio: p.PIO0,
        dio_pin: p.PIN_24,
        clk_pin: p.PIN_29,
        dma_ch: p.DMA_CH0,
    };
    spawner.spawn(time_updater(spawner, rtc, wifi_peripherals).unwrap());

    // Neopixel light effects
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 3_800_000;
    spi_config.phase = Phase::CaptureOnFirstTransition;
    spi_config.polarity = Polarity::IdleLow;
    let spi = Spi::new_txonly(p.SPI0, p.PIN_18, p.PIN_19, p.DMA_CH1, spi_config);
    spawner.spawn(task::light_effects::light_effects_handler(spi).unwrap());
}
