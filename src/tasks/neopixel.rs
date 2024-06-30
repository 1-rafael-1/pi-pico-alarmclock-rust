use crate::drivers::ws2812::{self, Ws2812};
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::dma::{AnyChannel, Channel};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{
    Common, Config, FifoJoin, Instance, InterruptHandler, Pio, PioPin, ShiftConfig, ShiftDirection,
    StateMachine,
};
use embassy_rp::{bind_interrupts, clocks, into_ref, Peripheral, PeripheralRef};
use embassy_time::{Duration, Instant, Ticker, Timer};
use fixed::types::U24F8;
use fixed_macro::fixed;
use smart_leds::RGB8;
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::task]
async fn analog_clock(_spawner: Spawner) {}

const S: usize = 0;
const N: usize = 16;

#[embassy_executor::task]
pub async fn alarm_sequence(_spawner: Spawner, np_ring: Ws2812<'_, P, S, N>) {
    info!("Start");
    // let p = embassy_rp::init(Default::default());

    // let Pio {
    //     mut common, sm0, ..
    // } = Pio::new(p.PIO0, irqs);

    // This is the number of leds in the string. Helpfully, the sparkfun thing plus and adafruit
    // feather boards for the 2040 both have one built in.
    const NUM_LEDS: usize = 16;
    let mut data = [RGB8::default(); NUM_LEDS];

    // initialize the neopixel ring
    // let mut np_ring = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_28);

    // // Loop forever making RGB values and pushing them out to the WS2812.
    // let mut ticker = Ticker::every(Duration::from_millis(10));
    // loop {
    //     for j in 0..(256 * 5) {
    //         debug!("New Colors:");
    //         for i in 0..NUM_LEDS {
    //             data[i] = wheel((((i * 256) as u16 / NUM_LEDS as u16 + j as u16) & 255) as u8);
    //             debug!("R: {} G: {} B: {}", data[i].r, data[i].g, data[i].b);
    //         }
    //         ws2812.write(&data).await;

    //         ticker.next().await;
    //     }
    // }

    let mut ticker = Ticker::every(Duration::from_millis(1000));
    let brightness = 30;
    loop {
        // simumlate a sunrise: start with all leds off, then slowly add leds while all leds that are already used slowly change color from red to warm white
        // sunrise
        info!("Sunrise");
        let start_color = RGB8::new(255, 0, 0); // red
        let end_color = RGB8::new(255, 250, 244); // morning daylight
        let color_transition_delay = 0.3;
        let start_brightness = 0;
        let end_brightness = 200;
        let duration_secs: u64 = 60; // seconds
        let start_time = Instant::now();

        np_ring.set_all_leds_off(&mut data).await;
        np_ring.write(&data).await;

        // loop for duration seconds
        while Instant::now() - start_time < Duration::from_secs(duration_secs) {
            // calculate the current brightness and color based on the elapsed time
            let elapsed_time = Instant::now() - start_time;
            let remaining_time = Duration::from_secs(duration_secs) - elapsed_time;
            let fraction_elapsed = elapsed_time.as_secs() as f32 / duration_secs as f32;
            let current_brightness =
                255 - (remaining_time.as_secs() as f32 / duration_secs as f32 * 255.0) as u8;
            let current_color: RGB8;
            if fraction_elapsed < color_transition_delay {
                current_color = start_color;
            } else {
                current_color = RGB8::new(
                    ((end_color.r as f32 - start_color.r as f32) * fraction_elapsed
                        + start_color.r as f32) as u8,
                    ((end_color.g as f32 - start_color.g as f32) * fraction_elapsed
                        + start_color.g as f32) as u8,
                    ((end_color.b as f32 - start_color.b as f32) * fraction_elapsed
                        + start_color.b as f32) as u8,
                );
            }

            // let current_color = RGB8::new(
            //     start_color.r + ((end_color.r as i16 - start_color.r as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
            //     start_color.g + ((end_color.g as i16 - start_color.g as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
            //     start_color.b + ((end_color.b as i16 - start_color.b as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
            // );
            // calculate the number of leds to light up based on the elapsed time, min 1, max NUM_LEDS
            let current_leds =
                (((fraction_elapsed * NUM_LEDS as f32) as usize) + 1).clamp(1, NUM_LEDS);

            info!(
                "Current brightness: {}, Current leds: {}, Current color {} {} {}",
                current_brightness, current_leds, current_color.r, current_color.g, current_color.b
            );

            // set the leds
            for i in 0..current_leds {
                np_ring
                    .set_led_color_and_brightness(&mut data, i, current_color, current_brightness)
                    .await;
            }
            // write the leds
            np_ring.write(&data).await;
            Timer::after(Duration::from_millis(100)).await;
        }

        ticker.next().await;
    }
}
