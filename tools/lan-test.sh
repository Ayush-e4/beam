#!/usr/bin/env bash
# The phone (on USB) and a `beam` CLI on this Mac, over the real Wi-Fi: the phone pairs with the
# Mac's code, sends it a file, then receives one from it. Prints both sides' speeds.
#
#   tools/lan-test.sh            # 64 MiB each way
#   MIB=256 tools/lan-test.sh
#   MAC_IP=192.168.1.20 ADB_SERIAL=XXXX tools/lan-test.sh
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
mib=${MIB:-64}
mac_port=47123
phone_port=47124
phone_name="Phone under test"
test_class=app.beam.core.lan.MacPeerOnDeviceTest
runner=app.beam.core.test/androidx.test.runner.AndroidJUnitRunner
adb_bin=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
adb=("$adb_bin" ${ADB_SERIAL:+-s "$ADB_SERIAL"})

work=$(mktemp -d)
cleanup() {
    local status=$?
    jobs -p | xargs kill 2>/dev/null || true
    wait 2>/dev/null || true
    if [[ $status == 0 ]]; then
        rm -rf "$work"
    else
        echo "lan-test: logs kept in $work" >&2
    fi
}
trap cleanup EXIT
export BEAM_HOME="$work/home"

fail() {
    echo "lan-test: $*" >&2
    exit 1
}

# Waits up to $3 seconds for a line starting with $2 in file $1, and prints it.
wait_for() {
    local file=$1 word=$2 seconds=$3
    for _ in $(seq $((seconds * 10))); do
        if grep -m1 "^$word" "$file" 2>/dev/null; then
            return 0
        fi
        sleep 0.1
    done
    fail "no '$word' within ${seconds}s in $(basename "$file"):$(echo; cat "$file")"
}

route_interface=$(route -n get default 2>/dev/null | awk '/interface:/ { print $2 }')
mac_ip=${MAC_IP:-$(ipconfig getifaddr "${route_interface:-en0}" || true)}
[[ -n $mac_ip ]] || fail "no address on ${route_interface:-en0}; set MAC_IP"
phone_ip=$("${adb[@]}" shell ip -4 addr show wlan0 | awk '/inet / { sub(/\/.*/, "", $2); print $2 }')
[[ -n $phone_ip ]] || fail "the phone has no Wi-Fi address"
echo "Mac $mac_ip, phone $phone_ip, $mib MiB each way"

studio_java="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
if [[ -z ${JAVA_HOME:-} && -d $studio_java ]]; then
    export JAVA_HOME=$studio_java
fi
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Library/Android/sdk}

echo "Building the CLI and the phone test…"
cargo build -q --release --manifest-path "$repo/core/Cargo.toml" -p beam-cli
beam="$repo/core/target/release/beam"
(cd "$repo/android" && ./gradlew -q :core:assembleDebugAndroidTest)
"${adb[@]}" install -r -t "$repo/android/core/build/outputs/apk/androidTest/debug/core-debug-androidTest.apk" >/dev/null

"$beam" pair --name Mac --address "$mac_ip" >"$work/pair.log" 2>&1 &
pairing=$!
code=$(wait_for "$work/pair.log" "CODE " 20 | cut -d' ' -f2)

"${adb[@]}" logcat -c
"${adb[@]}" logcat -v raw -s BeamMacPeer:I >"$work/phone.log" &
"${adb[@]}" shell am instrument -w -e class "$test_class" \
    -e macCode "$code" -e macHost "$mac_ip" -e macPort "$mac_port" -e mib "$mib" \
    "$runner" >"$work/instrument.log" 2>&1 &
instrument=$!

wait "$pairing" || fail "pairing failed:$(echo; cat "$work/pair.log")"
mac_paired=$(wait_for "$work/pair.log" "PAIRED" 1)
"$beam" receive --port "$mac_port" --into "$work/inbox" >"$work/receive.log" 2>&1 &
phone_paired=$(wait_for "$work/phone.log" "LAN PAIRED" 60)
verify() { sed -E 's/.*verify=([0-9]+( [0-9]+)*).*/\1/' <<<"$1"; }
[[ $(verify "$mac_paired") == "$(verify "$phone_paired")" ]] ||
    fail "verification codes differ: '$mac_paired' vs '$phone_paired'"

wait_for "$work/phone.log" "LAN SENT" 300 >/dev/null
wait_for "$work/receive.log" "RECEIVED" 30 >/dev/null
received=$(stat -f %z "$work/inbox/from-phone.bin")
[[ $received == $((mib * 1024 * 1024)) ]] || fail "the Mac got $received bytes"

wait_for "$work/phone.log" "LAN LISTENING" 30 >/dev/null
head -c $((mib * 1024 * 1024)) /dev/urandom >"$work/from-mac.bin"
started=$(date +%s)
"$beam" send "$phone_name" "$phone_ip" "$phone_port" "$work/from-mac.bin" >"$work/send.log" 2>&1 ||
    fail "the Mac's send failed:$(echo; cat "$work/send.log")"
wait "$instrument" || true
grep -q "OK (1 test)" "$work/instrument.log" ||
    fail "the phone test failed:$(echo; cat "$work/instrument.log")"

echo "Paired: both sides show $(verify "$mac_paired")"
echo "Phone → Mac: $(grep "^LAN SENT" "$work/phone.log" | cut -d' ' -f3-)"
echo "Mac → phone: $(grep "^LAN RECEIVED" "$work/phone.log" | cut -d' ' -f3-) (Mac side: $(($(date +%s) - started))s)"
