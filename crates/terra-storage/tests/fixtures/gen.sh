#!/bin/sh
# Regenerates the scanner's audio fixtures: one second of a mono 440 Hz sine
# wave in each format the scanner tests read, with synthetic tags.
#
# Run it from anywhere; it writes next to itself. It needs ffmpeg with
# libmp3lame and libopus. The tests read the committed files and never run
# ffmpeg; they change tags with Lofty instead.
set -eu

cd "$(dirname "$0")"

# Bit-exact output leaves out the encoder's version, so that regenerating
# with the same ffmpeg gives the same bytes.
tone() {
    out=$1
    shift
    ffmpeg -nostdin -hide_banner -loglevel error -y \
        -f lavfi -i "sine=frequency=440:duration=1:sample_rate=$RATE" \
        -ac 1 -fflags +bitexact -flags:a +bitexact -map_metadata -1 \
        "$@" "$out"
}

RATE=48000
tone tone.mp3 -c:a libmp3lame -b:a 32k \
    -metadata title="Tone MP3" -metadata artist="Artist A" \
    -metadata album="Album A" -metadata track=1 -metadata date=2001

RATE=8000
tone tone.flac -c:a flac \
    -metadata title="Tone FLAC" -metadata artist="Artist A" \
    -metadata album="Album A" -metadata track=2 -metadata date=2001

RATE=48000
tone tone.opus -c:a libopus -b:a 16k \
    -metadata title="Tone Opus" -metadata artist="Artist B" \
    -metadata album="Album B" -metadata track=1 -metadata date=2002

RATE=48000
tone tone-aac.m4a -c:a aac -b:a 32k \
    -metadata title="Tone AAC" -metadata artist="Artist B" \
    -metadata album="Album B" -metadata track=2 -metadata date=2002

RATE=8000
tone tone-alac.m4a -c:a alac \
    -metadata title="Tone ALAC" -metadata artist="Artist C" \
    -metadata album="Album C" -metadata track=1 -metadata disc=1 \
    -metadata date=2003
