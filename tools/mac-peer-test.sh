#!/usr/bin/env bash
# The phone (on USB) and a `beam` CLI on this Mac, over a real network: the phone pairs with the
# Mac's code, sends it a file, then receives one from it. Prints both sides' speeds.
#
#   tools/mac-peer-test.sh             # over the Wi-Fi both are on, 64 MiB each way
#   tools/mac-peer-test.sh --hotspot   # the phone opens its hotspot and the Mac joins it
#   MIB=256 MAC_IP=192.168.1.20 ADB_SERIAL=XXXX tools/mac-peer-test.sh
#
# With --hotspot the Mac leaves its Wi-Fi network (and the internet) for the test, then turns
# Wi-Fi off and on again so it rejoins its usual network.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
hotspot=false
[[ ${1:-} == --hotspot ]] && hotspot=true
mib=${MIB:-64}
mac_port=47123
phone_port=47124
phone_name="Phone under test"
test_package=app.beam.core.test
test_class=app.beam.core.lan.MacPeerOnDeviceTest
code_file=/sdcard/Android/data/$test_package/files/mac-code.txt
adb_bin=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
adb=("$adb_bin" ${ADB_SERIAL:+-s "$ADB_SERIAL"})
wifi_device=$(networksetup -listallhardwareports | awk '/Wi-Fi/ { getline; print $2 }')
joined=""

work=$(mktemp -d)
cleanup() {
    local status=$?
    jobs -p | xargs kill 2>/dev/null || true
    wait 2>/dev/null || true
    "${adb[@]}" shell am force-stop "$test_package" 2>/dev/null || true
    [[ -n $joined ]] && rejoin_usual_wifi
    if [[ $status == 0 ]]; then
        rm -rf "$work"
    else
        echo "mac-peer-test: logs kept in $work" >&2
    fi
}
trap cleanup EXIT
export BEAM_HOME="$work/home"

fail() {
    echo "mac-peer-test: $*" >&2
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

# The rest of a `LAN <word> …` line from the phone.
phone_says() {
    wait_for "$work/phone.log" "LAN $1" "$2" | cut -d' ' -f3-
}

join_hotspot() {
    local ssid=$1 password=$2 subnet=${3%.*}.
    echo "Joining the phone's hotspot ($ssid)…"
    joined=$ssid
    networksetup -setairportnetwork "$wifi_device" "$ssid" "$password" >"$work/join.log" 2>&1
    for _ in $(seq 300); do
        mac_ip=$(ipconfig getifaddr "$wifi_device" || true)
        [[ $mac_ip == "$subnet"* ]] && return 0
        sleep 0.1
    done
    fail "the Mac didn't get an address on the hotspot:$(echo; cat "$work/join.log")"
}

rejoin_usual_wifi() {
    echo "Rejoining the usual Wi-Fi…"
    # macOS saves every network it joins; forgetting one needs an administrator.
    if ! networksetup -removepreferredwirelessnetwork "$wifi_device" "$joined" >/dev/null 2>&1; then
        echo "mac-peer-test: to forget the hotspot: sudo networksetup" \
            "-removepreferredwirelessnetwork $wifi_device '$joined'" >&2
    fi
    networksetup -setairportpower "$wifi_device" off
    sleep 1
    networksetup -setairportpower "$wifi_device" on
    for _ in $(seq 60); do
        route -n get default >/dev/null 2>&1 && return 0
        sleep 0.5
    done
    echo "mac-peer-test: the Mac hasn't rejoined a network; check its Wi-Fi" >&2
}

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

"${adb[@]}" logcat -c
"${adb[@]}" logcat -v raw -s BeamMacPeer:I >"$work/phone.log" &
"${adb[@]}" shell am instrument -w -e class "$test_class" -e macPeer true \
    -e hotspot "$hotspot" -e mib "$mib" "$test_package/androidx.test.runner.AndroidJUnitRunner" \
    >"$work/instrument.log" 2>&1 &
instrument=$!

if [[ $hotspot == true ]]; then
    phone_ip=$(phone_says HOTSPOT 30 | sed -E 's/.*address=([0-9.]+).*/\1/')
    [[ $phone_ip =~ ^[0-9.]+$ ]] || fail "the phone's hotspot has no address"
    ssid=$(phone_says SSID 5)
    password=$(phone_says PASS 5)
    join_hotspot "$ssid" "$password" "$phone_ip"
else
    route_interface=$(route -n get default 2>/dev/null | awk '/interface:/ { print $2 }')
    mac_ip=${MAC_IP:-$(ipconfig getifaddr "${route_interface:-en0}" || true)}
    [[ -n $mac_ip ]] || fail "no address on ${route_interface:-en0}; set MAC_IP"
    phone_ip=$("${adb[@]}" shell ip -4 addr show wlan0 | awk '/inet / { sub(/\/.*/, "", $2); print $2 }')
    [[ -n $phone_ip ]] || fail "the phone has no Wi-Fi address"
fi
echo "Mac $mac_ip, phone $phone_ip, $mib MiB each way"
phone_says READY 30 >/dev/null

"$beam" pair --name Mac --address "$mac_ip" >"$work/pair.log" 2>&1 &
pairing=$!
code=$(wait_for "$work/pair.log" "CODE " 20 | cut -d' ' -f2)
printf '%s\n%s\n%s\n' "$code" "$mac_ip" "$mac_port" >"$work/mac-code.txt"
"${adb[@]}" push "$work/mac-code.txt" "$code_file" >/dev/null 2>&1

wait "$pairing" || fail "pairing failed:$(echo; cat "$work/pair.log")"
mac_paired=$(wait_for "$work/pair.log" "PAIRED" 1)
"$beam" receive --port "$mac_port" --into "$work/inbox" >"$work/receive.log" 2>&1 &
phone_paired=$(phone_says PAIRED 60)
verify() { sed -E 's/.*verify=([0-9]+( [0-9]+)*).*/\1/' <<<"$1"; }
[[ $(verify "$mac_paired") == "$(verify "$phone_paired")" ]] ||
    fail "verification codes differ: '$mac_paired' vs '$phone_paired'"

phone_says SENT 300 >/dev/null
wait_for "$work/receive.log" "RECEIVED" 30 >/dev/null
received=$(stat -f %z "$work/inbox/from-phone.bin")
[[ $received == $((mib * 1024 * 1024)) ]] || fail "the Mac got $received bytes"

phone_says LISTENING 30 >/dev/null
head -c $((mib * 1024 * 1024)) /dev/urandom >"$work/from-mac.bin"
"$beam" send "$phone_name" "$phone_ip" "$phone_port" "$work/from-mac.bin" >"$work/send.log" 2>&1 ||
    fail "the Mac's send failed:$(echo; cat "$work/send.log")"
wait "$instrument" || true
grep -q "OK (1 test)" "$work/instrument.log" ||
    fail "the phone test failed:$(echo; cat "$work/instrument.log")"

echo "Paired: both sides show $(verify "$mac_paired")"
echo "Phone → Mac: $(grep "^LAN SENT" "$work/phone.log" | cut -d' ' -f3-)"
echo "Mac → phone: $(grep "^LAN RECEIVED" "$work/phone.log" | cut -d' ' -f3-)"
