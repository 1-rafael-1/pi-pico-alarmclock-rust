# Raspberry Pi Pico Alarmclock written in Rust

[![ci](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml)

Building a (hopefully working) alarmclock based on a Raspberry Pi Pico W written in Rust and using the Embassy framework.

This is a picture of the prototype on a breadboard, in a box with bits of hardware dangling on their wires. Not pretty, but before i build a proper one in its enclosure it must do:
![Working Prototype](images/prototype.png)

## Features

+ **DateTime Retrieval**:
    + DateTime is obtained through a web request to `worldtimeapi.org` on device startup and refreshed every 6 hours.

+ **Display Modes**:
    + **Normal Mode**:
        + Shows the time in hours and minutes using a custom-made set of number images modeled after a StarWars font.
        + Displays the date and day of the week as text.
        + Shows an image of a lightsaber to indicate whether the alarm is active.
        + Includes a battery indicator showing whether the device is powered by USB or battery, and if by battery, also indicates the charge level.
    + **Setting Mode**:
        + Displays the currently saved alarm time in hours and minutes.
        + Shows an indicator that the device is in setup mode.
    + **Menu Mode**:
        + Displays a menu offering options to put the device into standby or view system information (mostly measured power supply voltage) and voltage bounds.

+ **Neopixel Ring**:
    + A 16-LED Neopixel Ring is used for visual effects. In normal mode with the alarm not active, an analog clock is simulated with the hour indicated in red, the minutes in green, and the seconds in blue. Whenever the hands meet, their colors mix. The analog clock is shown as long as the alarm is not active. When the alarm is active the leds remain off until an alarm is triggered. See below.

+ **MP3 Module and Speaker**:
    + A MP3 module and a 3W speaker are used to play the Imperial March as the alarm tone.

+ **Power Supply**:
    + Power is supplied from a 18650 Li-ion battery or via USB. When on USB, the Li-ion battery is charged. The device immediately changes the display and performs a voltage measurement when switching between USB and battery power.

+ **Push Buttons**:
    + Three push buttons (green, blue, yellow) allow user interaction. Their actions depend on the system state:
        + **Normal Mode**:
            + Green toggles alarm active.
            + Blue enters alarm time setup.
            + Yellow enters menu.
        + **Alarm Time Setting Mode**:
            + Green increases hours, one per single press or continuously when holding the button down for more than a second.
            + Yellow increases minutes, one per single press or continuously when holding the button down for more than a second.
            + Blue saves the setting.
        + **Menu Mode**:
            + Green enters system info.
            + Blue enters device standby.
            + Yellow goes back to normal mode.
        + **System Info**:
            + Any button enters normal mode.
        + **Standby**:
            + Any button wakes the device.

+ **Alarm Trigger**:
    + When the alarm is triggered:
        + The Neopixel plays a sunrise effect, starting with morning-red light and gradually adding more LEDs, changing all LED colors towards warm white light. When that is concluded, a whirling rainbow effect is played until the alarm state is left.
        + As soon as the sunrise effect on the Neopixel is done, the alarm sound plays the Imperial March exactly one time. It is a long song, and after extensive testing, I am thoroughly fed up with it.
        + The device randomizes a sequence of buttons and displays text in the state area to "Press Yellow!" or one of the other two. The user must press the requested color until all three buttons have been pressed. If the user does not press the correct sequence, the alarm will continue.

+ **Device Standby**:
    + When entering Standby mode the display and the neopixel ring are turned off. Internally the scheduler task, the time updater task and the voltage measuring task are suspended. That way no activity is performed and the device powers down as much as the Pi Pico W allows for, besides circuit loss.
    + Pressing any button in Standby mode will wake the device. All tasks resume, and one initial call to the time service is made.

## Code

The project is written in Rust making heavy use of the Embassy framework. I have attempted to document the code extensively, mainly because writing explanations is what I do when I learn new things.

The general layout of the project is as follows:

+ The module `tasks` contains crates for the async tasks that make up the system.
    + In this module the system state is described by `state.rs`.
    + Peripheral resources are defined in `resource.rs`.
    + The orchestration of the system is defined in `orchestrate.rs` where a scheduler task and an orchestrate task manage all system state changes.
    + Events and Commands for use throughout the tasks and the orchestrator are defined in `task_messages.rs`.
    + All other files define sepcific peripheral or system tasks.
+ The module `utility` is very small and defines some helper functions mainly for converting DateTime to and from String.
+ The folder `media` contains `bmp`-files used by the display task. These I made myself pixel by pixel, none of this is a copy.
+ The folder `wifi-firmware`contains the firmware for the wifi-chip, copied over from the Embassy repo for convenience.

To get the docs clone this repo and run this:

```Shell
cargo doc --open
```

## Building the Project

This project uses `defmt` for logging, which can be configured to include different log levels depending on whether you're building for development or production use. Log levels are controlled using the `DEFMT_LOG` environment variable at compile time.

### Debug Build

For development with all logging enabled (trace, debug, info, warn):

```Shell
cargo build
cargo run
```

This will include all `info!`, `debug!`, and `trace!` log statements, which are useful during development when connected to a debug probe. The logs are output via RTT (Real-Time Transfer) to your debugger.

### Release Build

For production use, you should build with reduced logging to prevent the RTT buffer from filling up and causing the device to hang when running standalone (without a debugger connected):

**Recommended: Warnings only**

```Shell
DEFMT_LOG=warn cargo build --release
DEFMT_LOG=warn cargo run --release
```

This will compile out all `info!` and `debug!` statements, keeping only `warn!` logs. This is the recommended configuration for flashing to a device that will run standalone.

**Info and above (moderate logging)**

```Shell
DEFMT_LOG=info cargo build --release
```

This keeps `info!` and `warn!` logs but removes `debug!` and `trace!`.

### Flashing Manually

To flash the device manually without a debug probe (note: `elf2uf2-rs` does not work with Rust versions 1.89 and up):

```Shell
# Build for release (optimized for size and power)
DEFMT_LOG=warn cargo build --release

# Option 1: Flash directly with picotool
# Put board in bootloader mode (hold BOOTSEL while connecting USB)
picotool load -u -v -x -t elf target/thumbv6m-none-eabi/release/pi-pico-alarmclock

# Option 2: Convert to UF2 and copy manually
picotool uf2 convert target/thumbv6m-none-eabi/release/pi-pico-alarmclock -t elf pi-pico-alarmclock.uf2 -t uf2
# Copy the resulting .uf2 file to the RPI-RP2 drive that appears when holding BOOTSEL during USB connection
```

## Testing

For testing during development, use the debug build with a debug probe connected to see all logs in real-time.

## Circuit

The circuit schematic can be found in KiCad format at [circuit/pi-pico-alarmclock/](circuit/pi-pico-alarmclock/). The design includes power management with a battery charger module, voltage level sensing, MOSFET switching for display and audio control, and all necessary peripheral connections.

## Enclosure

The enclosure is designed in Autodesk Fusion, a project Export of the design can be found here: [enclosure](enclosure/).

A gallery of images can be found [here](enclosure/gallery.md).

## Assembly

This is still WIP, I have my first pair of burns to show for it, really not good at soldering... Will update when done.

## Components (Bill of Materials)

### Main Components
|Component|Qty|Description|
|---------|---|-----------|
|Raspberry Pi Pico W|1|Microcontroller with WiFi|
|OLED Display|1|SSD1306 compatible I²C OLED Display 128×64 pixels with two color yellow/blue. Input Voltage 3.3V|
|DFPlayer Mini|1|MP3 module (DFR0299)|
|TC4056A Charger Module|1|Li-ion battery charging module with protection|
|Step-up Converter|1|5V boost converter (e.g., U3V16F5 or similar), 2.5-5.5V input, 5V/1A output|
|WS2812B NeoPixel Ring|1|16 RGB LED ring (this is the limit the power supply can handle)|
|Speaker|1|3W 8Ω speaker, 70×30×15mm (DFPlayer Mini compatible)|
|18650 Li-ion Battery|1|3350mAh or similar capacity|
|Battery Holder|1|For 18650 cell|
|Power Switch|1|Simple on/off switch|
|Micro SD Card|1|Any capacity, formatted to FAT32|

### Push Buttons
|Component|Qty|Description|
|---------|---|-----------|
|LED Ring Push Buttons|3|16mm LED ring illuminated push buttons with JST 4-pin connectors (one each: yellow, green, blue). LED rings are controlled via MOSFET Q4 (IRLZ44N) on GPIO 26 with 10-second auto-off timeout on button press (except during alarm mode)|

### Semiconductors
|Component|Qty|Type|Description|
|---------|---|----|-----------| 
|Q1, Q4|2|IRLZ44N|N-channel MOSFET, logic-level (TO-220)|
|Q3|1|IRF9540|P-channel MOSFET, logic-level (TO-220)|
|D3|1|1N5819|Schottky diode, 40V, 1A (DO-41)|

### Resistors (1/4W)
|Reference|Qty|Value|
|---------|---|-----|
|R1, R5|2|10kΩ|
|R2|1|100Ω|
|R3, R7|2|1kΩ|
|R6|1|2.2kΩ|
|R8|1|220Ω|
|R9, R11|2|680kΩ|
|R10, R12|2|1MΩ|

### Capacitors
|Reference|Qty|Type|Value|Voltage|
|---------|---|----|-----|-------|
|C1, C3, C4, C6|4|Ceramic|100nF|50V|
|C2, C7|2|Electrolytic|470µF|16V|

### Connectors
|Reference|Qty|Type|Description|
|---------|---|----|-----------|
|J1|1|Screw Terminal 2P|5mm pitch, for speaker connection|
|J2|1|JST PH 4-pin|Button Green (2mm pitch)|
|J3|1|JST PH 4-pin|Button Blue (2mm pitch)|
|J4|1|JST PH 4-pin|Button Yellow (2mm pitch)|
|J5|1|JST PH 3-pin|NeoPixel connection (2mm pitch)|
|J6|1|JST PH 4-pin|OLED display (2mm pitch)|
|J7|1|Screw Terminal 2P|5mm pitch, for battery connection|

### Miscellaneous
|Component|Description|
|---------|-----------|
|Wires|Various gauges for connections|
|Header pins|For Pico W if using socket mount|

## Disclaimer and Thanks

This is a hobby project and I have very little experience in electronics and had none before in Rust and also none before in Fusion. All three things i taught myself along the way. While this was incredible fun, this project will be full of imperfections, literally everywhere. In case You happen across this repo and spots a thing to improve - if You find the time to let me know, I will be more than happy. After all, this was and is about learning things.

That being said: This device does work, at least as far as I did test it to this point.

Does the world need another alarmclock? Hell no, it does not. You can buy them in thousands of types for very little money and then most will have more functionality, better battery life, and whatnot. I was looking for a thing to do, had a joking conversation with my eldest daughter (who is in an age range where getting up in the morning appears to be terribly difficult) and that was that: I found myself building this thing.

While doing this I had a ton of help, and I am very sure this would have ended nowhere without:

+ [Embassy framework](https://github.com/embassy-rs/embassy): This is a Rust framework for embedded devices, with PACs for an number of different chips and boards and packed with great features focusing on async multitasking. The maintainers have piled up  - and that was really helpful to me - an impressive number of examples on how to do connect devices as well as conceptual stuff on how to solve diverse things. I am glad I could contribute back some examples to that to give back a little.
+ Embassy Community: While getting to grips with Rust and Embassy some very kind and patient individuals from the Embassy Community helped me with my questions, which were a mix of Rust-rookie questions and Embassy-rookie questions. That was an amazing experience, and I clearly would have either not managed or at the very least needed ages without.

We should also not forget, that in this day and age it is a lot easier to learn a new programming language, because we have AI help. In my case I found it helpful to use GitHub Copilot, although it does have an evil twist sometimes, because Rust has not the training data other more prevalent languages have. Rust embedded is then an even smaller subset of that, further degrading response quality. So good prompting is key, and even so the stupid thing keeps suggesting using Tokio, Serde, ... and many std-things. But still, you can always ask about concepts, see the suggestions, even if often technically wrong but often conceptually still helpful... it does speed up things considerably. I believe I would have managed without, but at a fraction of the speed.
