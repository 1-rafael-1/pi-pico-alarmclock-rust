# pi-pico-alarmclock-rust

[![ci](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/1-rafael-1/pi-pico-alarmclock-rust/actions/workflows/ci.yml)

Building a (hopefully working) alarmclock based on a Raspberry Pi Pico W written in Rust and using the Embassy framework.


## Disclaimer and Thanks

This is a hobby project and I have very little experience in electronics and had none before in Rust and also none before in Fusion. All three things i taught myself along the way. While this was incredible fun, this project will be full of imperfections, literally everywhere. In case You happen across this repo and spots a thing to improve - if You find the time to let me know, I will be more than happy. After all, this was and is about learning things.

That being said: This device does work, at least as far as I did test it to this point. 

Does the world need another alarmclock? Hell no, it does not. You can buy them in thousands of types for very little money and then most will have more functionality, better battery life, and whatnot. I was looking for a thing to do, had a joking conversation with my eldest daughter (who is in an age range where getting up in the morning appears to be terribly difficult) and that was that: I found myself building this thing.

While doing this I had a ton of help, and I am very sure this would have ended nowhere without:

+ [Embassy framework](https://github.com/embassy-rs/embassy): This is a Rust framework for embedded devices, with PACs for an number of different chips and boards and packed with great features focusing on async multitasking. The maintainers have piled up  - and that was really helpful to me - an impressive number of examples on how to do connect devices as well as conceptual stuff on how to solve diverse things. I am glad I could contribute back some examples to that to give back a little.
+ Embassy Community: While getting to grips with Rust and Embassy some very kind and patient individuals from the Embassy Community helped me with my questions, which were a mix of Rust-rookie questions and Embassy-rookie questions. That was an amazing experience, and I clearly would have either not managed or at the very least needed ages without.

We should also not forget, that in this day and age it is a lot easier to learn a new programming language, because we have AI help. In my case I found it helpful to use GitHub Copilot, although it does have an evil twist sometimes, because Rust has not the training data other more prevalent laguages have. Rust embedded is then an even smaller subset of that, further degrading response quality. So good prompting is key, and even so the stupid thing keeps suggesting using Tokio, Serde, ... and many std-things. But still, you can always ask about concepts, see the suggestions, even if often technically wrong but often conceptually still helpful... it does speed up things considerably. I believe I would have managed without, but at a fraction of the speed.

## The Device

This is an alarmclock built around a Raspberry Pi Pico W. 

![Working Prototype](images/prototype.png)

The functionality is as follows:

+ DateTime is obtained through a webrequest to `worldtimeapi.org`, on device startup and after that with a refresh every 6 hours.
+ A Display shows (in normal mode):
    + the time in hours and minuts, using a custom-made set of number images, that I moldeled after some StarWars font.
    + the date and day of the week as text.
    + an image of a light-saber to indicate, if the alarm is active or not.
    + a battery indicator, showing either that the device is powered by USB or by battery, and if by battery also indicates the charge level
    + When in Setting mode the display shows the currently saved alarm time in hours and minutes and an indicator as to that we are in setup mode.
    + When in Menu Mode the display shows a menu, offering to put the device into standby or see some system information (mostly measured power supply voltage) and voltage bounds.
+ A 16-Leds Neopixel Ring is used for visual effects. In normal mode with the alarm not active a analog clock is simulated, the hour indicated red, the minutes green and the seconds blue. Whenever hands meet, their colors mix.
+ A mp3-module and a 3W-Speaker is used to play the Imperial March as the alarm tone.
+ Power is supplied from a 18650 LiIon battery or via USB, when on USB the LiIon is charged. When attaching/diattaching USB Power the device immediately changes the display, does a voltage measurement if on battery power.
+ Three push buttons (green, blue, yellow) allow user interaction. Their actions depend on the system state:
    + in normal mode green toggles alarm active, blue enters alarm time setup and yellow enters menu
    + in alarm time setting mode green increases hours, yellow increases minutes adn blue saves the setting
    + in menu mode green enters system info, blue enters device standby and yellow goes back to normal mode
    + in system info any buttin enters normal mode
    + in standby any button wakes the device
+ When the alarm is triggered 
    + the neopixel plays a sunrise effect, starting with morning-red light and adding more and more leds slowly changing all led colors towards warm white light. When that is concluded a whirling ranibow effect is played until the alarm state is left.
    + as soon as the sunrise effect on the neopixel is done, the alarm sound plays the Imperial March exactly one time. It is a long song, and after testing my ass of I am throughly fed up with it.
    + The device randomizes a sequence of buttons and on the display in the state area shows text to "Press Yellow!" or one of teh other two. The user must then proceed to press the requestetd color until all three buttons have been pressed. That being donem the alarm is stopped.

## Circuit

![Circuit Diagram](circuit/circuit.png)