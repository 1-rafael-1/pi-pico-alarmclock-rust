use crate::task::resources::{UsbPowerResources, VsysPowerResources};
use defmt::*;
use embassy_executor::Spawner;

#[embassy_executor::task]
pub async fn usb_power(spawner: Spawner, r: UsbPowerResources) {
    info!("usb_power task started");
    loop {}
}
