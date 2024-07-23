//! # Power
//! Determine the power state of the system: battery or power supply.
//! Detremine the supply voltage of the system.

use crate::task::resources::{Irqs, UsbPowerResources};
use crate::task::state::VBUS_CHANNEL;
use crate::VsysResources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::adc::{Adc, Channel, Config};
use embassy_rp::gpio::{Input, Pull};
use embassy_time::{Duration, Timer};

/// determine the power source of the system, specifically if the USB power supply is connected
/// the USB power supply is connected, if the pin is high
/// Note: We are using a voltage divider to detect the USB power supply through a GPIO pin. Due to the intricacies of the Pico W,
/// the VBUS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vbus concurrently).
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

/// measure the voltage of the Vsys rail
/// this is either the battery voltage or the usb power supply voltage, if the usb power supply is connected.
/// Note: We are using a voltage divider to measure the Vsys voltage through a GPIO pin. Due to the intricacies of the Pico W,
/// the VSYS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vsys concurrently).
#[embassy_executor::task]
pub async fn vsys_voltage(_spawner: Spawner, r: VsysResources) {
    info!("vsys_voltage task started");
    let mut adc = Adc::new(r.adc, Irqs, Config::default());
    let vsys_in = r.pin_27;
    let mut channel = Channel::new_pin(vsys_in, Pull::None);
    let refresh_after_secs = 600; // 10 minutes
    loop {
        // read the adc value
        let adc_value = adc.read(&mut channel).await.unwrap();
        let voltage = (adc_value as f32) * 3.3 * 3.0 / 4096.0;

        info!(
            "vsys_voltage: adc_value: {}, voltage: {}",
            adc_value, voltage
        );
        Timer::after(Duration::from_secs(refresh_after_secs)).await;
    }
}
