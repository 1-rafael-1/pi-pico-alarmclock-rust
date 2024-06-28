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

pub struct Ws2812<'d, P: Instance, const S: usize, const N: usize> {
    dma: PeripheralRef<'d, AnyChannel>,
    sm: StateMachine<'d, P, S>,
}

impl<'d, P: Instance, const S: usize, const N: usize> Ws2812<'d, P, S, N> {
    pub fn new(
        pio: &mut Common<'d, P>,
        mut sm: StateMachine<'d, P, S>,
        dma: impl Peripheral<P = impl Channel> + 'd,
        pin: impl PioPin,
    ) -> Self {
        into_ref!(dma);

        // Setup sm0

        // prepare the PIO program
        let side_set = pio::SideSet::new(false, 1, false);
        let mut a: pio::Assembler<32> = pio::Assembler::new_with_side_set(side_set);

        const T1: u8 = 2; // start bit
        const T2: u8 = 5; // data bit
        const T3: u8 = 3; // stop bit
        const CYCLES_PER_BIT: u32 = (T1 + T2 + T3) as u32;

        let mut wrap_target = a.label();
        let mut wrap_source = a.label();
        let mut do_zero = a.label();
        a.set_with_side_set(pio::SetDestination::PINDIRS, 1, 0);
        a.bind(&mut wrap_target);
        // Do stop bit
        a.out_with_delay_and_side_set(pio::OutDestination::X, 1, T3 - 1, 0);
        // Do start bit
        a.jmp_with_delay_and_side_set(pio::JmpCondition::XIsZero, &mut do_zero, T1 - 1, 1);
        // Do data bit = 1
        a.jmp_with_delay_and_side_set(pio::JmpCondition::Always, &mut wrap_target, T2 - 1, 1);
        a.bind(&mut do_zero);
        // Do data bit = 0
        a.nop_with_delay_and_side_set(T2 - 1, 0);
        a.bind(&mut wrap_source);

        let prg = a.assemble_with_wrap(wrap_source, wrap_target);
        let mut cfg = Config::default();

        // Pin config
        let out_pin = pio.make_pio_pin(pin);
        cfg.set_out_pins(&[&out_pin]);
        cfg.set_set_pins(&[&out_pin]);

        cfg.use_program(&pio.load_program(&prg), &[&out_pin]);

        // Clock config, measured in kHz to avoid overflows
        // TODO CLOCK_FREQ should come from embassy_rp
        let clock_freq = U24F8::from_num(clocks::clk_sys_freq() / 1000);
        let ws2812_freq = fixed!(800: U24F8);
        let bit_freq = ws2812_freq * CYCLES_PER_BIT;
        cfg.clock_divider = clock_freq / bit_freq;

        // FIFO config
        cfg.fifo_join = FifoJoin::TxOnly;
        cfg.shift_out = ShiftConfig {
            auto_fill: true,
            threshold: 24,
            direction: ShiftDirection::Left,
        };

        sm.set_config(&cfg);
        sm.set_enable(true);

        Self {
            dma: dma.map_into(),
            sm,
        }
    }

    pub async fn write(&mut self, colors: &[RGB8; N]) {
        // Precompute the word bytes from the colors
        let mut words = [0u32; N];
        for i in 0..N {
            let word = (u32::from(colors[i].g) << 24)
                | (u32::from(colors[i].r) << 16)
                | (u32::from(colors[i].b) << 8);
            words[i] = word;
        }

        // DMA transfer
        self.sm.tx().dma_push(self.dma.reborrow(), &words).await;

        Timer::after_micros(55).await;
    }
}

/// Input a value 0 to 255 to get a color value
/// The colours are a transition r - g - b - back to r.
fn wheel(mut wheel_pos: u8) -> RGB8 {
    wheel_pos = 255 - wheel_pos;
    if wheel_pos < 85 {
        return (255 - wheel_pos * 3, 0, wheel_pos * 3).into();
    }
    if wheel_pos < 170 {
        wheel_pos -= 85;
        return (0, wheel_pos * 3, 255 - wheel_pos * 3).into();
    }
    wheel_pos -= 170;
    (wheel_pos * 3, 255 - wheel_pos * 3, 0).into()
}

/// Function to set a single LED's color and brightness
async fn set_led_color_and_brightness(
    data: &mut [RGB8],
    index: usize,
    color: RGB8,
    brightness: u8,
) {
    // Check if index is within bounds
    if index > data.len() {
        return;
    }

    // Adjust color based on brightness
    let adjusted_color = RGB8 {
        r: (color.r as u16 * brightness as u16 / 255) as u8,
        g: (color.g as u16 * brightness as u16 / 255) as u8,
        b: (color.b as u16 * brightness as u16 / 255) as u8,
    };
    data[index] = adjusted_color;
}

async fn set_led_off(data: &mut [RGB8], index: usize) {
    set_led_color_and_brightness(data, index, RGB8::default(), 0).await;
}

async fn set_all_leds_off(data: &mut [RGB8]) {
    for i in 0..data.len() {
        set_led_off(data, i).await;
    }
}

// #[embassy_executor::main]
// async fn main(_spawner: Spawner) {
//     info!("Start");
//     let p = embassy_rp::init(Default::default());

//     let Pio {
//         mut common, sm0, ..
//     } = Pio::new(p.PIO0, Irqs);

//     // This is the number of leds in the string. Helpfully, the sparkfun thing plus and adafruit
//     // feather boards for the 2040 both have one built in.
//     const NUM_LEDS: usize = 16;
//     let mut data = [RGB8::default(); NUM_LEDS];

//     // Common neopixel pins:
//     // Thing plus: 8
//     // Adafruit Feather: 16;  Adafruit Feather+RFM95: 4
//     let mut ws2812 = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_28);

//     // // Loop forever making RGB values and pushing them out to the WS2812.
//     // let mut ticker = Ticker::every(Duration::from_millis(10));
//     // loop {
//     //     for j in 0..(256 * 5) {
//     //         debug!("New Colors:");
//     //         for i in 0..NUM_LEDS {
//     //             data[i] = wheel((((i * 256) as u16 / NUM_LEDS as u16 + j as u16) & 255) as u8);
//     //             debug!("R: {} G: {} B: {}", data[i].r, data[i].g, data[i].b);
//     //         }
//     //         ws2812.write(&data).await;

//     //         ticker.next().await;
//     //     }
//     // }

//     let mut ticker = Ticker::every(Duration::from_millis(1000));
//     let brightness = 30;
//     loop {
//         // // Set all leds off
//         // set_all_leds_off(&mut data).await;
//         // ws2812.write(&data).await;

//         // ticker.next().await;

//         // // Set all leds to red at 50% brightness
//         // for i in 0..NUM_LEDS {
//         //     set_led_color_and_brightness(&mut data, i, RGB8::new(255, 0, 0), brightness).await;
//         // }
//         // ws2812.write(&data).await;

//         // ticker.next().await;

//         // // Set all leds to green at 50% brightness
//         // for i in 0..NUM_LEDS {
//         //     set_led_color_and_brightness(&mut data, i, RGB8::new(0, 255, 0), brightness).await;
//         // }
//         // ws2812.write(&data).await;

//         // ticker.next().await;

//         // // Set all leds to blue at 50% brightness
//         // for i in 0..NUM_LEDS {
//         //     set_led_color_and_brightness(&mut data, i, RGB8::new(0, 0, 255), brightness).await;
//         // }
//         // ws2812.write(&data).await;

//         // ticker.next().await;

//         // // Set all leds to white at 50% brightness
//         // for i in 0..NUM_LEDS {
//         //     set_led_color_and_brightness(&mut data, i, RGB8::new(255, 255, 255), brightness).await;
//         // }
//         // ws2812.write(&data).await;

//         // ticker.next().await;

//         // // let a red pixel chase the tail of a green pixel
//         // for i in 0..NUM_LEDS {
//         //     set_led_off(&mut data, i).await;
//         // }
//         // for i in 0..NUM_LEDS {
//         //     set_led_color_and_brightness(&mut data, (i + 1) % NUM_LEDS, RGB8::new(0, 255, 0),brightness).await;
//         //     set_led_color_and_brightness(&mut data, i, RGB8::new(255, 0, 0),brightness).await;
//         //     ws2812.write(&data).await;
//         //     Timer::after(Duration::from_millis(100)).await;
//         // }

//         // ticker.next().await;

//         // simumlate a sunrise: start with all leds off, then slowly add leds while all leds that are already used slowly change color from red to warm white
//         // sunrise
//         info!("Sunrise");
//         let start_color = RGB8::new(255, 0, 0); // red
//         let end_color = RGB8::new(255, 250, 244); // morning daylight
//         let color_transition_delay = 0.3;
//         let start_brightness = 0;
//         let end_brightness = 200;
//         let duration_secs: u64 = 60; // seconds
//         let start_time = Instant::now();

//         set_all_leds_off(&mut data).await;
//         ws2812.write(&data).await;

//         // loop for duration seconds
//         while Instant::now() - start_time < Duration::from_secs(duration_secs) {
//             // calculate the current brightness and color based on the elapsed time
//             let elapsed_time = Instant::now() - start_time;
//             let remaining_time = Duration::from_secs(duration_secs) - elapsed_time;
//             let fraction_elapsed = elapsed_time.as_secs() as f32 / duration_secs as f32;
//             let current_brightness =
//                 255 - (remaining_time.as_secs() as f32 / duration_secs as f32 * 255.0) as u8;
//             let current_color: RGB8;
//             if fraction_elapsed < color_transition_delay {
//                 current_color = start_color;
//             } else {
//                 current_color = RGB8::new(
//                     ((end_color.r as f32 - start_color.r as f32) * fraction_elapsed
//                         + start_color.r as f32) as u8,
//                     ((end_color.g as f32 - start_color.g as f32) * fraction_elapsed
//                         + start_color.g as f32) as u8,
//                     ((end_color.b as f32 - start_color.b as f32) * fraction_elapsed
//                         + start_color.b as f32) as u8,
//                 );
//             }

//             // let current_color = RGB8::new(
//             //     start_color.r + ((end_color.r as i16 - start_color.r as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
//             //     start_color.g + ((end_color.g as i16 - start_color.g as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
//             //     start_color.b + ((end_color.b as i16 - start_color.b as i16) as f32 / duration_secs as f32 * elapsed_time.as_secs() as f32) as u8,
//             // );
//             // calculate the number of leds to light up based on the elapsed time, min 1, max NUM_LEDS
//             let current_leds =
//                 (((fraction_elapsed * NUM_LEDS as f32) as usize) + 1).clamp(1, NUM_LEDS);

//             info!(
//                 "Current brightness: {}, Current leds: {}, Current color {} {} {}",
//                 current_brightness, current_leds, current_color.r, current_color.g, current_color.b
//             );

//             // set the leds
//             for i in 0..current_leds {
//                 set_led_color_and_brightness(&mut data, i, current_color, current_brightness).await;
//             }
//             // write the leds
//             ws2812.write(&data).await;
//             Timer::after(Duration::from_millis(100)).await;
//         }

//         ticker.next().await;
//     }
// }
