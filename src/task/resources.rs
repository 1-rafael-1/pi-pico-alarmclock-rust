//! # Resources
//! this module is used to define the resources that will be used in the tasks
//!
//! the resources are defined in the main.rs file, and assigned to the tasks in the main.rs file
use assign_resources::assign_resources;
use embassy_rp::adc::InterruptHandler as AdcInterruptHandler;
use embassy_rp::i2c::InterruptHandler as I2cInterruptHandler;
use embassy_rp::peripherals::UART1;
use embassy_rp::peripherals::{I2C0, PIO0};
use embassy_rp::pio::InterruptHandler;
use embassy_rp::rtc::InterruptHandler as RtcInterruptHandler;
use embassy_rp::uart::BufferedInterruptHandler;
use embassy_rp::{bind_interrupts, peripherals, Peri};

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
    real_time_clock: RtcResources {
        rtc: RTC,
    },
    neopixel: NeopixelResources {
        inner_spi: SPI0,
        clk_pin: PIN_18, // this is just a dummy pin, the neopixel uses only the mosi pin
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
    vbus_power: UsbPowerResources {
        // we cannot use the VBUS power pin 24, because on the Pico W the vbus pin is run through the wifi module and is not available
        // instead we wire a voltage divider between VBUS and a GPIO pin
        vbus_pin: PIN_28,
    },
    wifi: WifiResources {
        pwr_pin: PIN_23,
        cs_pin: PIN_25,
        pio_sm: PIO0,
        dio_pin: PIN_24,
        clk_pin: PIN_29,
        dma_ch: DMA_CH0,
    },
    vsys_resources: VsysResources {
        // we cannot use the VSYS power pin 29, because on the Pico W the vsys pin is run through the wifi module and is not available
        // instead we wire a voltage divider between VSYS and a GPIO pin
        adc: ADC,
        pin_27: PIN_27,
    },
    flash: FlashResources {
        dma_ch: DMA_CH4,
        flash: FLASH,
    }
}

// bind the interrupts, on a global scope for convenience
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
    UART1_IRQ => BufferedInterruptHandler<UART1>;
    ADC_IRQ_FIFO => AdcInterruptHandler;
    RTC_IRQ => RtcInterruptHandler;
});
