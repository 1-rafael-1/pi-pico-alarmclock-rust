use assign_resources::assign_resources;
use embassy_rp::i2c::{InterruptHandler as I2cInterruptHandler};
use embassy_rp::peripherals::PIO0;
use embassy_rp::peripherals::{I2C0};
use embassy_rp::pio::{InterruptHandler};
use embassy_rp::{bind_interrupts, peripherals};

// Assign the resources to the peripherals
assign_resources! {
    btn_green: ButtonResourcesGreen {
        button_pin: PIN_20,
    },
    btn_blue: ButtonResourcesBlue {
        button_pin: PIN_21,
    },
    btn_yellow: ButtonResourcesYellow {
        button_pin: PIN_22,
    },
    wifi: WifiResources {
        pwr_pin: PIN_23,
        cs_pin: PIN_25,
        pio_sm: PIO0,
        dio_pin: PIN_26,
        clk_pin: PIN_27,
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
}

// bind the interrupts, on a global scope, until i find a better way
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
});
