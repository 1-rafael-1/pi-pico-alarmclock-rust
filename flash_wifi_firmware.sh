#!/bin/sh

# Navigate to the cyw43-firmware directory
cd "$(dirname "$0")/wifi-firmware/cyw43-firmware" || exit 1

# Download firmware using probe-rs
echo "Downloading firmware..."
probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
echo "Firmware download completed."
