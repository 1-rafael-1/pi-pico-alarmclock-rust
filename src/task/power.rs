use crate::task::resources::UsbPowerResources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Pull};

#[embassy_executor::task]
pub async fn usb_power(_spawner: Spawner, r: UsbPowerResources) {
    info!("usb_power task started");
    let mut vbus_in = Input::new(r.vbus_pin, Pull::None);
    loop {
        vbus_in.wait_for_any_edge().await;
    }
}
