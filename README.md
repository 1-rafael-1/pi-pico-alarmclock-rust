# pi-pico-alarmclock-rust

Building a (hopefully working) alarmclock based on a Raspberry Pi Pico W.

This repository is my attempt to recreate my MicroPython project in Rust. Read up on the project in the original Repo: 

[Pi Pico Alarmclock](https://github.com/1-rafael-1/pi-pico-alarmclock) -> including wiring, enclosure and general stuff.

I have never done anything in or with Rust before besides reading the Rust book (and dropping out halfway through), so this attempt here is more about inexpertly cobbling together things from examples, GitHub Copilot and googling. I hope to learn more about rust along the way, but be warned: Tons of imperfections ahead.

I aim at slowly adding component after component in Rust. I am not at all sure if I will ever complete this :-)

## Why?

Because I feel that MicroPython is fun and easy and a great language to do things quickly and with confidence. But at some level of complexity, one starts to really miss a debugger... in the MicroPython Project I have never managed to resolve my UART issue with the DFPlayer mini, and no amount of googling got me even close to a solution. Since I am only doing all this on my own time and for my own fun, why the heck not start over in Rust? There is no way I can waste my time here, even if nothing ever gets completed, I will have learned a lot and will have had a lot of fun.

Why Rust then? -> Maybe it is madness... but to be honest it was fairly easy to get the toolchain installed. It felt quite a bit like out of the box, and an evening spent to get the C/C++ toolchain to behave on my Windows machine, even using the Raspberry Pi Foundations Documents for just that - ended nowhere.

If that sounds familiar, have a look at either [Embassy](https://github.com/embassy-rs/embassy) and/or [https://github.com/rp-rs](https://github.com/rp-rs/rp-hal). Especially the Embassy framework is very vibrant, but both offer project templates and the community around bth is very active, and very helpful.

## How?

For the moment I have settled on Embassy to rely on mostly. I am using their examples as starting points and attempt to make everything asynch tasks. That suits me well, because I happen to already have most things running on timers and interrupts in my MicroPython project.

I have opted out of trying to use a BSP (board support package) on top of Embassy, for the moment it feels like the HAL (hardware abstraction layer) supplied as part of Embassy is enough to get going.

Besides that I really only fiddle things into working. That I do by trying, failing, Copiloting, Googling, ... I know, Rust is not made for that and in all honesty I likely could not achieve anything without the given Framework examples and modern tooling to help me. But then again, this is for fun... so who cares. 