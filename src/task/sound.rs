//! # Sound task
//!  This module contains the task that plays sound using the DFPlayer Mini module.
//!
//! The task is responsible for initializing the DFPlayer Mini module, powering it on, playing a sound, and powering it off.
use crate::task::resources::{DfPlayerResources, Irqs};
use crate::task::task_messages::{SOUND_START_SIGNAL, SOUND_STOP_SIGNAL};
use defmt::{info, Debug2Format};
use dfplayer_serial::{DfPlayer, Equalizer, PlayBackSource};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::uart::{BufferedUart, Config};
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn sound_handler(_spawner: Spawner, r: DfPlayerResources) {
    info!("Sound task started");

    let mut config = Config::default();
    config.baudrate = 9600;

    let mut tx_buffer = [0; 256];
    let mut rx_buffer = [0; 256];

    let mut uart = BufferedUart::new(
        r.uart,
        Irqs,
        r.tx_pin,
        r.rx_pin,
        &mut tx_buffer,
        &mut rx_buffer,
        config,
    );

    let feedback_enable = false; // fails to acknoweledge when enabled
    let timeout = Duration::from_secs(1);
    let reset_duration_override = Some(Duration::from_millis(1000));

    // power pin, not a part of the dfplayer, using a mosfet to control power to the dfplayer because it draws too much current when idle
    let mut pwr = Output::new(r.power_pin, Level::Low);

    loop {
        // wait for the signal to start playing sound
        SOUND_START_SIGNAL.wait().await;

        // power on the dfplayer
        info!("Powering on the dfplayer");
        pwr.set_high();
        Timer::after(Duration::from_secs(1)).await;
        info!("Powered on the dfplayer");

        let mut dfp_result =
            DfPlayer::try_new(&mut uart, feedback_enable, timeout, reset_duration_override).await;

        match dfp_result {
            Ok(_) => info!("DfPlayer initialized successfully"),
            Err(ref e) => info!(
                "DfPlayer initialization failed with error {:?}",
                Debug2Format(&e)
            ),
        }

        info!("Playing sound");
        if let Ok(ref mut dfp) = dfp_result {
            let _ = dfp.volume(13).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.equalizer(Equalizer::Classic).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.playback_source(PlayBackSource::SDCard).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.play(1).await;
            Timer::after(Duration::from_millis(200)).await;
        } else {
            info!("DfPlayer not initialized, skipping sound playback.");
        }

        // wait for the signal to stop playing sound
        SOUND_STOP_SIGNAL.wait().await;

        // power off the dfplayer
        info!("Powering off the dfplayer");
        pwr.set_low();
    }
}
