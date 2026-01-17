#!/bin/bash
#
# Build Rustica OS Live Bootable Image
#
# This script creates a GPT disk image with:
# - FAT32 EFI System Partition (compact, actual size)
# - BOOTX64.EFI bootloader at /EFI/BOOT/
# - kernel.efi at /EFI/Rustux/
#

set -e

VERSION="0.1.0"
IMAGE_NAME="rustica-live-amd64-${VERSION}.img"
SYMLINK_NAME="rustica-live-amd64.img"
KERNEL_DIR="/var/www/rustux.com/prod/kernel/kernel-efi"
LOADER_DIR="/var/www/rustux.com/prod/kernel/uefi-loader"
OUTPUT_DIR="/var/www/rustux.com/html/rustica"
MOUNT_POINT="/tmp/rustica-mount"

# Image size: 128MB (UEFI requires ESP >= 100MB)
# This creates a non-sparse image with actual file size
IMAGE_SIZE_MB=128

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "==================================================================="
echo "  Rustica OS Live Image Builder v${VERSION}"
echo "  Compact image (actual file size, no sparse)"
echo "==================================================================="
echo ""

# Step 1: Build kernel
echo -e "${YELLOW}[1/6] Building kernel...${NC}"
cargo build --release --manifest-path "${KERNEL_DIR}/Cargo.toml" --target x86_64-unknown-uefi
echo -e "${GREEN}✓ Kernel built${NC}"
echo ""

# Step 2: Build bootloader
echo -e "${YELLOW}[2/6] Building bootloader...${NC}"
cargo build --release --manifest-path "${LOADER_DIR}/Cargo.toml" --target x86_64-unknown-uefi
echo -e "${GREEN}✓ Bootloader built${NC}"
echo ""

# Step 3: Copy binaries to staging area
echo -e "${YELLOW}[3/6] Staging binaries...${NC}"
STAGING_DIR=$(mktemp -d)
mkdir -p "${STAGING_DIR}/EFI/BOOT"
mkdir -p "${STAGING_DIR}/EFI/Rustux"

# Copy bootloader (as BOOTX64.EFI)
cp "${LOADER_DIR}/target/x86_64-unknown-uefi/release/rustux-uefi-loader.efi" \
   "${STAGING_DIR}/EFI/BOOT/BOOTX64.EFI"
echo -e "${GREEN}  ✓ Bootloader: EFI/BOOT/BOOTX64.EFI${NC}"

# Copy kernel
cp "${KERNEL_DIR}/target/x86_64-unknown-uefi/release/rustux-kernel-efi.efi" \
   "${STAGING_DIR}/EFI/Rustux/kernel.efi"
echo -e "${GREEN}  ✓ Kernel: EFI/Rustux/kernel.efi${NC}"

# Calculate staging size
STAGING_SIZE=$(du -sk "${STAGING_DIR}" | cut -f1)
echo -e "${GREEN}  Staging size: ${STAGING_SIZE} KB${NC}"
echo ""

# Step 4: Create disk image (actual size, not sparse)
echo -e "${YELLOW}[4/6] Creating disk image...${NC}"
IMAGE_PATH="${OUTPUT_DIR}/${IMAGE_NAME}"

# Remove old image if exists
if [ -f "${IMAGE_PATH}" ]; then
    rm -f "${IMAGE_PATH}"
    echo "  Removed old image"
fi

# Create actual-sized image (not sparse)
# We use 128MB to meet UEFI ESP requirement (>= 100MB)
dd if=/dev/zero of="${IMAGE_PATH}" bs=1M count=${IMAGE_SIZE_MB} 2>&1 | grep -v "records"
echo -e "${GREEN}  ✓ Created ${IMAGE_SIZE_MB}MB image (actual size, UEFI bootable)${NC}"
echo ""

# Step 5: Create GPT partition table and FAT32 ESP
echo -e "${YELLOW}[5/6] Creating partitions...${NC}"

# Create GPT partition table
# Partition starts at 1MB (2048 sectors) and uses the rest of the image
parted -s "${IMAGE_PATH}" mklabel gpt
parted -s "${IMAGE_PATH}" mkpart primary FAT32 1MiB 100%
parted -s "${IMAGE_PATH}" set 1 esp on

# Setup loop device
LOOP_DEV=$(losetup -f --show "${IMAGE_PATH}")
echo "  Loop device: ${LOOP_DEV}"

# Probe partitions
partprobe "${LOOP_DEV}" 2>/dev/null || true
sleep 1

# Format partition as FAT32 with 16MB size
# This creates a filesystem that fits our needs
mkfs.vfat -F 32 -n "RUSTICA" "${LOOP_DEV}p1" > /dev/null 2>&1
echo -e "${GREEN}  ✓ Created FAT32 ESP${NC}"

# Mount partition
mkdir -p "${MOUNT_POINT}"
mount "${LOOP_DEV}p1" "${MOUNT_POINT}"
echo "  Mounted at ${MOUNT_POINT}"

# Copy files
cp -r "${STAGING_DIR}"/* "${MOUNT_POINT}/"
sync
echo -e "${GREEN}  ✓ Copied files to image${NC}"

# Show actual usage
FINAL_USAGE=$(du -sk "${MOUNT_POINT}" | cut -f1)
echo -e "${GREEN}  Final usage: ${FINAL_USAGE} KB${NC}"

# Unmount
umount "${MOUNT_POINT}"
losetup -d "${LOOP_DEV}"
rm -rf "${MOUNT_POINT}"
echo -e "${GREEN}  ✓ Unmounted and detached${NC}"
echo ""

# Step 6: Create symlink and checksum
echo -e "${YELLOW}[6/6] Finalizing...${NC}"

# Remove old symlink
cd "${OUTPUT_DIR}"
rm -f "${SYMLINK_NAME}"
ln -s "${IMAGE_NAME}" "${SYMLINK_NAME}"
echo -e "${GREEN}  ✓ Created symlink: ${SYMLINK_NAME}${NC}"

# Generate SHA256 checksum
sha256sum "${IMAGE_NAME}" > "${IMAGE_NAME}.sha256"
echo -e "${GREEN}  ✓ Generated SHA256 checksum${NC}"

# Cleanup staging
rm -rf "${STAGING_DIR}"
echo ""

# Verification
echo "==================================================================="
echo -e "${GREEN}✓ Build complete!${NC}"
echo "==================================================================="
echo ""
echo "Image created: ${IMAGE_PATH}"
echo ""

# Display verification info
echo "Verification:"
echo "-------------"
ls -lh "${IMAGE_NAME}" | awk '{print "  File size: " $5}'
du -h "${IMAGE_PATH}" | awk '{print "  Disk usage: " $1}'
echo ""
echo "Partition table:"
fdisk -l "${IMAGE_PATH}" 2>&1 | grep -E "Disk|EFI"
echo ""

echo "SHA256 Checksum:"
cat "${IMAGE_NAME}.sha256"
echo ""

echo -e "${GREEN}Image ready for download or USB writing!${NC}"
echo ""
echo "Note: This is a ${IMAGE_SIZE_MB}MB image with actual file size (not sparse)."
echo "      UEFI requires ESP >= 100MB for proper boot detection."
