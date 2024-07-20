use crate::task::resources::{
    Irqs, UsbPowerResources, VsysPowerResources, WifiVsysPins, WIFI_VSYS_PINS,
};
use crate::task::state::VBUS_CHANNEL;
use core::borrow::BorrowMut;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::adc::{self, Adc, Channel, Config, InterruptHandler};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{ADC, PIN_25, PIN_29};
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn usb_power(_spawner: Spawner, r: UsbPowerResources) {
    info!("usb_power task started");
    let mut vbus_in = Input::new(r.vbus_pin, Pull::None);
    let sender = VBUS_CHANNEL.sender();
    loop {
        info!("usb_power task loop");
        sender.send(vbus_in.is_high().into()).await;
        vbus_in.wait_for_any_edge().await;
        info!("usb_power edge detected");
    }
}

#[embassy_executor::task]
pub async fn vsys_voltage(_spawner: Spawner, r: VsysPowerResources) {
    info!("vsys_voltage task started");

    let mut adc = Adc::new(r.adc, Irqs, Config::default());

    loop {
        // get pins from the mutex, locking them for the duration of the scope

        // define the mutex guard, that we will drop at the end of the scope
        let mut wifi_vsys_pins_guard = WIFI_VSYS_PINS.lock().await;

        // if the mutex guard is not empty, we can proceed, dropping the mutex guard at the end of the scope
        if let Some(ref mut wifi_vsys_pins) = *wifi_vsys_pins_guard {
            // cs_pin is required to facilitate reading adc values from vsys on a Pico W
            let cs_pin = wifi_vsys_pins.cs_pin.borrow_mut();
            let mut cs_output = Output::new(cs_pin, Level::Low);

            // vsys_clk_pin is required as the channel for the adc
            let clk_pin = wifi_vsys_pins.vsys_clk_pin.borrow_mut();
            let mut clk_channel = Channel::new_pin(clk_pin, Pull::None);

            // we need the adc in this scope, so we don't get value moved errors in the loop
            let adc = &mut adc;

            // for reading the adc value, pin 25 has to cycle through low to high, not sure why... but this is how it works
            cs_output.set_high();
            Timer::after(Duration::from_millis(20)).await;

            // read the adc value
            let adc_value = adc.read(&mut clk_channel).await.unwrap();
            let voltage = (adc_value as f32) * 3.3 * 3.0 / 4096.0;

            // for reading the adc value, pin 25 has to cycle through low to high, not sure why... but this is how it works
            cs_output.set_low();
            Timer::after(Duration::from_millis(20)).await;

            info!(
                "vsys_voltage: adc_value: {}, voltage: {}",
                adc_value, voltage
            );
        } else {
            info!("vsys_voltage no pins");
            return;
        }
        Timer::after(Duration::from_secs(30)).await;
    }
}
