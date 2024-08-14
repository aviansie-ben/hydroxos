.PHONY: clean run run-kdbg test-kernel test .EXTERNALDEPS
.EXTERNALDEPS:

build/kernel-debug.bin: .EXTERNALDEPS
	@ mkdir -p $(@D)
	@ cd kernel && CARGO_MANIFEST_DIR=. cargo bootimage
	@ cp -p kernel/target/x86_64-hydroxos-kernel/debug/bootimage-hydroxos_kernel.bin build/kernel-debug.bin

build/kernel-release.bin: .EXTERNALDEPS
	@ mkdir -p $(@D)
	@ cd kernel && CARGO_MANIFEST_DIR=. cargo bootimage --release
	@ cp -p kernel/target/x86_64-hydroxos-kernel/release/bootimage-hydroxos_kernel.bin build/kernel-release.bin

run: build/kernel-release.bin
	@ qemu-system-x86_64 -drive format=raw,file=build/kernel-release.bin -serial stdio -cpu qemu64,+xsave,+xsaveopt $$QEMU_OPTIONS

run-kdbg: build/kernel-debug.bin
	@ qemu-system-x86_64 -drive format=raw,file=build/kernel-debug.bin -serial stdio -cpu qemu64,+xsave,+xsaveopt $$QEMU_OPTIONS

test-kernel:
	@ cd kernel && cargo test --lib -- $$QEMU_OPTIONS

test: test-kernel

clean:
	@ cd kernel && cargo clean
	@ rm -rf build
