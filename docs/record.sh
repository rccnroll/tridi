#!/bin/bash
# Records a viewer GIF on niri (scale 2): docs/record.sh NAME "KEYS" FILE...
# KEYS: space-separated; a number with a dot (0.8) sleeps, anything else is
# typed with wtype. Needs gpu-screen-recorder, wtype, ffmpeg. Run it in the
# samples dir of docs/samples.py; the GIF lands next to this script:
#   docs/record.sh files "0.8 1 0.7 2 0.7 3 0.7 4 1.2 4 0.6 3 0.6 2 0.6 1 1.0" tile_*.pcd
#   CROP=0.6 docs/record.sh points "0.8 + 0.35 + 0.35 ... 1.2 - 0.35 - ... 1.0" knot.pcd
# wtype keys only after the floating toggle and resize below: on a window
# that hasn't changed, niri doesn't pass wtype's keymap, its keycode 1 reads
# as Esc and the viewer quits.
set -eu
S=$(dirname "$0"); name=$1 keys=$2; shift 2
W=1200 H=700 X=360 Y=250   # logical px, output at scale 2
"$S/../target/release/tridi" "$@" >/dev/null 2>&1 & P=$!
for i in $(seq 40); do
  ID=$(niri msg -j windows | python3 -c 'import json,sys;w=[x for x in json.load(sys.stdin) if x["app_id"]=="tridi"];print(w[0]["id"] if w else "")')
  [ -n "$ID" ] && break; sleep 0.5
done
niri msg action toggle-window-floating --id $ID
niri msg action set-window-width --id $ID $W; niri msg action set-window-height --id $ID $H
niri msg action move-floating-window --id $ID -x $X -y $Y
sleep 1; niri msg action focus-window --id $ID; wtype r; sleep 1
read -r PX PY < <(niri msg -j windows | python3 -c "import json,sys;w=[x for x in json.load(sys.stdin) if x['id']==$ID][0];p=w['layout']['tile_pos_in_workspace_view'];print(int(p[0]),int(p[1]))")
gpu-screen-recorder -w eDP-1 -f 30 -cursor no -o $S/$name.raw.mp4 >/dev/null 2>&1 & R=$!
sleep 1.5
for k in $keys; do
  if [[ $k == *.* ]]; then sleep $k; else niri msg action focus-window --id $ID; wtype "$k"; fi
done
sleep 1; kill -INT $R; wait $R || true; kill $P || true
# crop the window (physical px = 2x logical) and make a palette GIF at half size
ffmpeg -loglevel error -y -i $S/$name.raw.mp4 -vf "crop=$((W*2)):$((H*2)):$((PX*2)):$((PY*2)),crop=iw*${CROP:-0.72}:ih*(${CROP:-0.72}+0.12):(iw-ow)/2:(ih-oh)/2,fps=15,scale=${GW:-800}:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4" $S/$name.gif
rm -f $S/$name.raw.mp4
echo "$name.gif pos=$PX,$PY $(du -h $S/$name.gif | cut -f1)"
