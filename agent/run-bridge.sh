#!/bin/sh
# (Re)starts the bridge.
#
# The console server outlives a killed bridge and keeps holding the FUSE-T mount;
# a new bridge would then hang silently trying to mount the same path (output
# stops after "session at ...").
#
# So this script cleans up whatever a killed game or bridge left behind. (The
# other safety net is the bridge exiting on its own via STS2_EXIT_WITH_GAME.)
set -e
MOUNT="${STS2_MOUNT:-/tmp/sts2-mnt}"

# The mod may start us from any working directory.
cd "$(dirname "$0")"

# API keys: started by the mod there is no shell profile, so load .env here.
for ENV_FILE in ../../.env ../.env ./.env; do
    if [ -f "$ENV_FILE" ]; then
        set -a
        . "$ENV_FILE"
        set +a
        break
    fi
done

pkill -f 'target/release/sts2-bridge' 2>/dev/null || true
pkill -f cortex-local-console 2>/dev/null || true
sleep 1
umount "$MOUNT" 2>/dev/null || diskutil unmount force "$MOUNT" >/dev/null 2>&1 || true

exec ./target/release/sts2-bridge "$@"
