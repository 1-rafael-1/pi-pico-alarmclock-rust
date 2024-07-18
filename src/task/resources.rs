/// this module is used to define the resources that will be used in the tasks
///
/// the resources are defined in the main.rs file, and assigned to the tasks in the main.rs file
use crate::task::state::StateManager;
use assign_resources::assign_resources;
use embassy_rp::adc::{Adc, Channel, Config, InterruptHandler as AdcInterruptHandler};
use embassy_rp::i2c::InterruptHandler as I2cInterruptHandler;
use embassy_rp::peripherals::UART1;
use embassy_rp::peripherals::{I2C0, PIN_25};
use embassy_rp::peripherals::{PIN_29, PIO0};
use embassy_rp::pio::InterruptHandler;
use embassy_rp::uart::BufferedInterruptHandler;
use embassy_rp::{bind_interrupts, peripherals};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;

// group the peripherlas into resources, to be used in the tasks
// the resources are assigned to the tasks in main.rs
assign_resources! {
    btn_green: GreenButtonResources {
        button_pin: PIN_20,
    },
    btn_blue: BlueButtonResources {
        button_pin: PIN_21,
    },
    btn_yellow: YellowButtonResources {
        button_pin: PIN_22,
    },
    wifi: WifiResources {
        pwr_pin: PIN_23,
        // PIN_25 is the cs pin, this we handle through a mutex, see below
        //cs_pin: PIN_25,
        pio_sm: PIO0,
        dio_pin: PIN_24,
        // PIN_25 is the clk pin, this we handle through a mutex, see below
        //clk_pin: PIN_29,
        dma_ch: DMA_CH0,
    },
    rtc: RtcResources {
        rtc_inst: RTC,
    },
    neopixel: NeopixelResources {
        inner_spi: SPI0,
        clk_pin: PIN_18,
        mosi_pin: PIN_19,
        tx_dma_ch: DMA_CH1,
    },
    display: DisplayResources {
        scl: PIN_13,
        sda: PIN_12,
        i2c0: I2C0,
    },
    dfplayer: DfPlayerResources {
        uart: UART1,
        tx_pin: PIN_4,
        rx_pin: PIN_5,
        rx_dma_ch: DMA_CH2,
        tx_dma_ch: DMA_CH3,
        power_pin: PIN_8, // not a part of the dfplayer, using a mosfet to control power to the dfplayer because it draws too much current when idle
    },
    vbus: UsbPowerResources {
        // we cannot use the USB power pin 24, because on the Pico W the vbus pin is run through the wifi module and is not available
        // instead we wire a voltage divider between VBUS and a GPIO pin
        vbus_pin: PIN_27,
    },
}

// some resources are shared between tasks, so we need to wrap them in a mutex
// these are resources used by the wifi chip as well as power.rs
// the mutex is defined here, and the resources are assigned to the mutex in the main.rs file
pub struct VsysPins {
    pub cs_pin: PIN_25, // required to facilitate reading adc values from vsys on a Pi ico W
    pub vsys_pin: PIN_29,
}

pub type VsysPinsType = Mutex<ThreadModeRawMutex, Option<VsysPins>>;
pub static VSYS_PINS: VsysPinsType = Mutex::new(None);

// bind the interrupts, on a global scope, until i find a better way
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
    UART1_IRQ => BufferedInterruptHandler<UART1>;
    ADC_IRQ_FIFO => AdcInterruptHandler;
});

pub struct TaskConfig {
    pub spawn_btn_green: bool,
    pub spawn_btn_blue: bool,
    pub spawn_btn_yellow: bool,
    pub spawn_connect_and_update_rtc: bool,
    pub spawn_neopixel: bool,
    pub spawn_display: bool,
    pub spawn_dfplayer: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            spawn_btn_green: true,
            spawn_btn_blue: true,
            spawn_btn_yellow: true,
            spawn_connect_and_update_rtc: true,
            spawn_neopixel: true,
            spawn_display: true,
            spawn_dfplayer: true,
        }
    }
}

impl TaskConfig {
    pub fn new() -> Self {
        TaskConfig::default()
    }
}
