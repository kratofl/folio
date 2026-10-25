#!/bin/sh
# Prepares a checkout for work. Run it through `make setup`, which sets MAIN,
# FIXTURES, ZED_REF and GPUI_REV. Safe to run again.
#
# In a new worktree, the main checkout's build output is cloned in first, so
# the first build only compiles Folio's own crates instead of all of gpui.
# Cargo reuses the cloned dependencies because their fingerprints don't
# depend on where the checkout is.
set -eu

if [ "$(pwd -P)" != "$(cd "$MAIN" && pwd -P)" ] && [ ! -d target ] && [ -d "$MAIN/target" ]; then
    echo "Cloning the build output from $MAIN"
    if [ "$(uname)" = Darwin ]; then
        cp -cR "$MAIN/target" target
    else
        cp -R --reflink=auto "$MAIN/target" target
    fi
fi

cargo fetch --quiet

if [ ! -d "$FIXTURES" ]; then
    echo "Note: no fixture photos at $FIXTURES. make scratch and make screenshot need them."
fi
if [ ! -d "$ZED_REF" ]; then
    echo "Note: no Zed reference checkout at $ZED_REF (see the gpui skill)."
elif [ "$(git -C "$ZED_REF" rev-parse HEAD)" != "$GPUI_REV" ]; then
    echo "Note: $ZED_REF isn't at the pinned gpui commit $GPUI_REV."
fi
echo "Ready. make lists the commands."
