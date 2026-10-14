#!/usr/bin/env bash
# The generated GTK hook forces x11. Let GTK choose the native session, while
# preserving an explicitly requested backend (including an explicit x11 choice).
if [ "${REFLOW_APPIMAGE_ORIGINAL_GDK_BACKEND-__REFLOW_UNSET__}" = "__REFLOW_UNSET__" ]; then
    unset GDK_BACKEND
else
    export GDK_BACKEND="$REFLOW_APPIMAGE_ORIGINAL_GDK_BACKEND"
fi
