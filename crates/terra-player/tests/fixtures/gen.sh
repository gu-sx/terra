#!/bin/sh
# Regenerates the synthetic audio fixtures of terra-player's tests.
# Usage: sh tests/fixtures/gen.sh, from crates/terra-player.
#
# The method is that of the playback-engine spike
# (docs/decisions/0002-playback-engine.md, Appendix A.1), shortened from 20 s
# to 3 s so that the fixtures stay small. Every file is a generated tone; the
# tags are generic.
set -eu
cd "$(dirname "$0")"
ff="ffmpeg -nostdin -hide_banner -loglevel error -y -bitexact"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Gapless: a continuous 3 s, 1 kHz sine at 48 kHz, mono, amplitude 0.5, split
# at sample 72517, which is not a multiple of any codec's frame size (MP3
# 1152, AAC 1024, Opus 960, FLAC 4096, ALAC 4096). a.* holds samples 0..72517
# and b.* the rest, 71483 samples.
split=72517
$ff -f lavfi -i "aevalsrc=0.5*sin(2*PI*1000*t):s=48000:d=3" -c:a pcm_s16le "$tmp/sine.wav"
$ff -i "$tmp/sine.wav" -af "atrim=end_sample=$split" -c:a pcm_s16le "$tmp/a.wav"
$ff -i "$tmp/sine.wav" -af "atrim=start_sample=$split,asetpts=PTS-STARTPTS" -c:a pcm_s16le "$tmp/b.wav"

# Marker: the same tone, silent before sample 3000 and after 50999, so the
# onset shows how many samples an engine adds or drops at the start of a file.
$ff -f lavfi -i "aevalsrc=if(between(n\\,3000\\,50999)\\,0.5*sin(2*PI*1000*t)\\,0):s=48000:d=1.125" \
  -c:a pcm_s16le "$tmp/marker.wav"

encode() { # <name> <title>
  for codec in "libmp3lame -b:a 192k mp3" "flac flac" "libopus -b:a 128k opus" \
      "aac -b:a 192k aac.m4a" "alac alac.m4a"; do
    ext=${codec##* }
    $ff -i "$tmp/$1.wav" -metadata title="$2" -metadata artist="Artist A" \
      -metadata album="Album A" -c:a ${codec% *} "$1.$ext"
  done
}
encode a "Tone A"
encode b "Tone B"
encode marker "Marker"

# Seeking: 1 s of seeded pink noise, AAC in M4A only, so that a window after a
# seek matches exactly one position in the file (a sine repeats every 48
# samples). The seek test locates it in the file's own decode. Perceptual
# noise substitution is off: its bands decode from a random generator, so no
# two decodes would match.
$ff -f lavfi -i "anoisesrc=color=pink:seed=23:amplitude=0.5:sample_rate=48000:duration=1" \
  -c:a pcm_s16le "$tmp/noise.wav"
$ff -i "$tmp/noise.wav" -metadata title="Noise" -metadata artist="Artist A" \
  -metadata album="Album A" -c:a aac -b:a 128k -aac_pns 0 noise.aac.m4a
