use crate::task::resources::{DfPlayerResources, Irqs};
use dfplayer_serial::{DfPlayer, Equalizer, PlayBackSource};
use embassy_executor::Spawner;
use embassy_rp::uart::{
    BufferedUart, Config, DataBits, Parity, StopBits,
};
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn sound(_spawner: Spawner, r: DfPlayerResources) {
    let mut config = Config::default();
    config.baudrate = 9600;
    config.stop_bits = StopBits::STOP1;
    config.data_bits = DataBits::DataBits8;
    config.parity = Parity::ParityNone;

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

    let feedback_enable = true;
    let timeout = Duration::from_secs(1);
    let reset_duration_override = None;
    //    let dfp = DfPlayer::try_new(&mut uart, feedback_enable, timeout, reset_duration_override);
    // let mut dfp = DfPlayer::try_new(&mut uart, feedback_enable, timeout, reset_duration_override);

    let mut dfp_result =
        DfPlayer::try_new(&mut uart, feedback_enable, timeout, reset_duration_override).await;

    loop {
        if let Ok(ref mut dfp) = dfp_result {
            let _ = dfp.volume(30).await;
            let _ = dfp.equalizer(Equalizer::Classic).await;
            let _ = dfp.playback_source(PlayBackSource::SDCard).await;
            let _ = dfp.play(1).await;
        } else {
            // Handle the error appropriately
        }
        Timer::after(Duration::from_secs(60)).await;
    }
}
