# Raspberry Pi Pico Alarmclock written in Rust

[![ci](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml)

Building a (hopefully working) alarmclock based on a Raspberry Pi Pico W written in Rust and using the Embassy framework.

## The Device

This is an alarmclock built around a Raspberry Pi Pico W. The functionality is as follows:

+ DateTime is obtained through a web request to `worldtimeapi.org`, on device startup and after that with a refresh every 6 hours.
+ A Display shows (in normal mode):
    + the time in hours and minutes, using a custom-made set of number images, that I modeled after some StarWars font.
    + the date and day of the week as text.
    + an image of a lightsaber to indicate, whether the alarm is active or not.
    + a battery indicator, showing either that the device is powered by USB or by battery, and if by battery also indicates the charge level
    + When in Setting mode the display shows the currently saved alarm time in hours and minutes and an indicator as to that we are in setup mode.
    + When in Menu Mode the display shows a menu, offering to put the device into standby or see some system information (mostly measured power supply voltage) and voltage bounds.
+ A 16-Leds Neopixel Ring is used for visual effects. In normal mode with the alarm not active a analog clock is simulated, the hour indicated red, the minutes green and the seconds blue. Whenever the hands meet, their colors mix.
+ A MP2 module and a 3W speaker is used to play the Imperial March as the alarm tone.
+ Power is supplied from a 18650 Li-Ion battery or via USB, when on USB the Li-Ion is charged. When attaching/dis-attaching USB Power the device immediately changes the display, does a voltage measurement if on battery power.
+ Three push buttons (green, blue, yellow) allow user interaction. Their actions depend on the system state:
    + in normal mode green toggles alarm active, blue enters alarm time setup and yellow enters menu
    + in alarm time setting mode green increases hours, yellow increases minutes adn blue saves the setting
    + in menu mode green enters system info, blue enters device standby and yellow goes back to normal mode
    + in system info any button enters normal mode
    + in standby any button wakes the device
+ When the alarm is triggered 
    + the Neopixel plays a sunrise effect, starting with morning-red light and adding more and more leds slowly changing all led colors towards warm white light. When that is concluded a whirling rainbow effect is played until the alarm state is left.
    + as soon as the sunrise effect on the Neopixel is done, the alarm sound plays the Imperial March exactly one time. It is a long song, and after testing my ass of I am thoroughly fed up with it.
    + The device randomizes a sequence of buttons and on the display in the state area shows text to "Press Yellow!" or one of teh other two. The user must then proceed to press the requested color until all three buttons have been pressed. That being done the alarm is stopped.

This is a picture of the prototype on a breadboard, in a box with bits of hardware dangling on their wires. Not pretty, but before i build a proper one in its enclosure it must do: 
![Working Prototype](images/prototype.png)

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

## Testing

To compile and run via debug probe:

```Shell
cargo build --release
cargo run --release
```

To flash manually:

```Shell
cargo build --release
cargo install elf2uf2-rs
elf2uf2-rs .\target\thumbv6m-none-eabi\release\pi-pico-alarmclock
```

And then find the `uf2` file in the above folder and flash that manually to the Pi Pico.

As an alternative find the latest release and use the `uf2` file from there. 

## Circuit

This is my best attempt at a circuit diagram. Not knowing much about electronics and long-buried memories from school slowly re-loading from cold storage this was trial and error and a lot of googling before it worked. In this configuration i am reasonably sure it is okay to start soldering a first model.
![Circuit Diagram](circuit/circuit.png)

## Enclosure

The enclosure is designed in Autodesk Fusion, a project Export of the design can be found here: [enclosure](enclosure/). 

A gallery of images can be found [here](circuit/gallery.md).

## Assembly

This is still WIP, I have my first pair of burns to show for it, really not good at soldering... Will update when done.

## Components

|Component|Description|
|---------|---------|
|Microcontroller|Raspberry Pi Pico W|
|OLED Display|SSD1306 compatible I²C OLED Display 128*64 pixels with two color yellow/blue. Input Voltage 3.3V|
|battery|A 18650 Lithium-Ion battery with 3350mAh. Anything else will work, as long as it fits with the charger module and outputs no more than 5V.|
|battery holder|really anything will do|
|power switch|Any simple switch to cut power between the battery and the charger module|
|charger module|A TC4056A module here, but any similar module will work, as long as it can be powered by pads and fits the Li-Ion battery specs. A managed charger that protects the battery is preferred.|
|NeoPixel ring|WS2812B with 16 RGB LED on it. This is the limit on what the power supply can handle.|
|step-up converter|U3V16F5 used here. Any other converter will do, that can convert the expected input between 2.5V and 5V and convert that to a steady 5V with 1000mA.|
|speaker|DFplayer Mini 3 Watt 8Ω speaker, 70*30*15mm. They can be found in some flavors from multiple vendors. Depending on the form factor, not all will fit into the enclosure as designed here.|
|p-channel MOSFET|Two IRF9540 used here. Other models will do, as long as the gate voltage of 3.3V is sufficient to fully switch (look for "logic-level MOSFET") and they can handle 5V safely. A ton of options exist and the ones used here are probably not the most ideal choice.|
|n-channel MOSFET|One IRLZ44N used here. Other models will do, as long as the gate voltage of 3.3V is sufficient to fully switch (look for "logic-level MOSFET") and it can handle 5V safely. A ton of options exist and the one used here is probably not the most ideal choice.|
|Schottky diode|One used, anything rated for 5V will do.|
|mp3 module|DFR0299 (DFPlayer)|
|micro sd card|Whatever, formatted to FAT32.|
|push button|Three used. 13mm diameter, 8mm hight caps on 12x12x7.3mm button - these should be fairly standard. One caps each in yellow, green and blue.|
|Resistors|Three 1MΩ, Two 680KΩ and one 220Ω|
|Wires|Plenty :-)|

## Disclaimer and Thanks

This is a hobby project and I have very little experience in electronics and had none before in Rust and also none before in Fusion. All three things i taught myself along the way. While this was incredible fun, this project will be full of imperfections, literally everywhere. In case You happen across this repo and spots a thing to improve - if You find the time to let me know, I will be more than happy. After all, this was and is about learning things.

That being said: This device does work, at least as far as I did test it to this point. 

Does the world need another alarmclock? Hell no, it does not. You can buy them in thousands of types for very little money and then most will have more functionality, better battery life, and whatnot. I was looking for a thing to do, had a joking conversation with my eldest daughter (who is in an age range where getting up in the morning appears to be terribly difficult) and that was that: I found myself building this thing.

While doing this I had a ton of help, and I am very sure this would have ended nowhere without:

+ [Embassy framework](https://github.com/embassy-rs/embassy): This is a Rust framework for embedded devices, with PACs for an number of different chips and boards and packed with great features focusing on async multitasking. The maintainers have piled up  - and that was really helpful to me - an impressive number of examples on how to do connect devices as well as conceptual stuff on how to solve diverse things. I am glad I could contribute back some examples to that to give back a little.
+ Embassy Community: While getting to grips with Rust and Embassy some very kind and patient individuals from the Embassy Community helped me with my questions, which were a mix of Rust-rookie questions and Embassy-rookie questions. That was an amazing experience, and I clearly would have either not managed or at the very least needed ages without.

We should also not forget, that in this day and age it is a lot easier to learn a new programming language, because we have AI help. In my case I found it helpful to use GitHub Copilot, although it does have an evil twist sometimes, because Rust has not the training data other more prevalent languages have. Rust embedded is then an even smaller subset of that, further degrading response quality. So good prompting is key, and even so the stupid thing keeps suggesting using Tokio, Serde, ... and many std-things. But still, you can always ask about concepts, see the suggestions, even if often technically wrong but often conceptually still helpful... it does speed up things considerably. I believe I would have managed without, but at a fraction of the speed.
