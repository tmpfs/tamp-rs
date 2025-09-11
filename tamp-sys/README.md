# Build

## ESP32

For ESP32 with the `espup` toolchain installed:

```
cargo +esp build --target xtensa-esp32s3-none-elf  -Zbuild-std=core,alloc
```

## ARM Cortex M4

Install the dependencies, for example on Arch:

```
sudo pacman -S arm-none-eabi-newlib arm-none-eabi-gcc
```

Then check with:

```
arm-none-eabi-gcc -print-sysroot
```

Now you can build:

```
cargo build --target thumbv7em-none-eabihf
```
