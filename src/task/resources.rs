use assign_resources::assign_resources;
use embassy_rp::i2c::InterruptHandler as I2cInterruptHandler;
use embassy_rp::peripherals::I2C0;
use embassy_rp::peripherals::PIO0;
use embassy_rp::peripherals::UART1;
use embassy_rp::pio::InterruptHandler;
use embassy_rp::uart::BufferedInterruptHandler;
use embassy_rp::{bind_interrupts, peripherals};

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
        cs_pin: PIN_25,
        pio_sm: PIO0,
        dio_pin: PIN_24,
        clk_pin: PIN_29,
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
}

// bind the interrupts, on a global scope, until i find a better way
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
    // UART0_IRQ => UartInterruptHandler<UART0>;
    UART1_IRQ => BufferedInterruptHandler<UART1>;
});
