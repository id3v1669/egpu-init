# Why this exists
I have a PCB with an Oculink port connected to the M2 NVMe slot of the Lenovo Thinkpad L14 Gen 3 AMD.
Lenovo BIOS is a mess and does not properly train m2 NVME PCI slots.
It assigns busses to upstream/downstream, but does not reach the GPU as firmware does not scan/assign beyond 2 elements. Linux can fix that behaviour via flags `pci=assign-busses` to assign buses and `pci=realloc` to assign proper BARs, but macOS/hackintosh doesn't.
# Theory on how to fix that:
1) Mod bios - too complicated
2) Use coreboot - in progress, but requires too much time and knowledge, so a temporary solution is required
3) Create a UEFI app to init GPU and load it by the bootloader - **That is the one for now**
# Dev process
* Tried assigning busses to bus 3/4/5 as 3/4 were assigned by firmware, but that did not work.
* Assigning buses 10/11/12 worked and flag `pci=assign-busses` can be abandoned.
* To drop flag `pci=realloc`, we need to assign BARs; turns out that just initializing them was enough to drop the flag, as Linux rewrites those values when it sees garbage.
* Since I want to potentially make the GPU post in the future on the bootloader level, it is better to go as low as possible with busses and set correct values for BARs
* Lowest bars are 08/09/0a. Looks like busses 1-7 are locked/limited by firmware, and working with them would be a nightmare.
* Correct values for BARs are extracted from Linux `dmesg` command and set successfully.
* Exploring what else Linux does to the iGPU, we may need to apply the same logic to the eGPU. Potential actions are: Extended Tags, Power to D0 for each bus with a standard delay, Enable Command register (Memory, I/O, Bus Master).
* All busses already are D0, no power signals required.
* ExtTags are also already on, so no extra assignment required.
* Enable command register didn't do anything, but since it doesn't hurt, we will keep that code at least for now
* GPU works correctly in both Linux and MacOS at this point, but doesn't post the bootloader. Other values need to be reviewed for further development process.
* 2 GOPs are detected, one of them is attached to the GPU, another one is not assignedto anything.
* ROM signature valid (55 AA)
* Problem is that DLL(Data Link Layer) is down and for that reason MMIO is silent
* In Linux `dmesg` reports that the GPU starts posting when the amdgpu driver fetches VBIOS from the ROM BAR and loads the ATOM BIOS. To do that, the DLL must be on, but I didn't find it in `dmesg`.

# TODO:
* Figure out how Linux and macOS train PCI to bring DLL up.
* Replicate pci behaviour from Linux source code.
* If after the DLL is up, the GPU doesn't post, write ATOM BIOS interpreter