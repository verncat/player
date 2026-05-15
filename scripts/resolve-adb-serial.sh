#!/bin/sh
set -eu

if [ -n "${ANDROID_SERIAL:-}" ]; then
  printf '%s\n' "$ANDROID_SERIAL"
  exit 0
fi

device_serials=$(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')

if [ -z "$device_serials" ]; then
  echo "No adb devices found. Connect a device or set ANDROID_SERIAL." >&2
  exit 1
fi

preferred_serials=$(printf '%s\n' "$device_serials" | grep -F -v '._adb-tls-connect._tcp' || true)
preferred_count=$(printf '%s\n' "$preferred_serials" | sed '/^$/d' | wc -l | tr -d ' ')

if [ "$preferred_count" -eq 1 ]; then
  printf '%s\n' "$preferred_serials"
  exit 0
fi

if [ "$preferred_count" -gt 1 ]; then
  echo "Multiple adb devices found. Set ANDROID_SERIAL to choose one:" >&2
  printf '%s\n' "$preferred_serials" >&2
  exit 1
fi

device_count=$(printf '%s\n' "$device_serials" | sed '/^$/d' | wc -l | tr -d ' ')

if [ "$device_count" -eq 1 ]; then
  printf '%s\n' "$device_serials"
  exit 0
fi

echo "Multiple adb devices found. Set ANDROID_SERIAL to choose one:" >&2
printf '%s\n' "$device_serials" >&2
exit 1