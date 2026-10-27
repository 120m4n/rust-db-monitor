#!/bin/sh
# Regenera los iconos desde los SVG. Requiere rsvg-convert e ImageMagick (brew install librsvg imagemagick).
#   assets/db-monitor.ico          icono del .exe (16, 24 desde logo-small.svg; 32–256 desde logo.svg)
#   assets/tray/<estado>.png       iconos de bandeja 32×32 (vista y fuente)
#   assets/tray/<estado>.rgba      los mismos en RGBA crudo, que src/desktop.rs incluye con include_bytes!
set -eu
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

for z in 16 24; do rsvg-convert -w $z -h $z logo-small.svg -o "$T/$z.png"; done
for z in 32 48 64 256; do rsvg-convert -w $z -h $z logo.svg -o "$T/$z.png"; done
magick "$T/16.png" "$T/24.png" "$T/32.png" "$T/48.png" "$T/64.png" "$T/256.png" db-monitor.ico

# bandeja: cuerpo con el color de estado, tapa del mismo color aclarado; iris en blanco para no competir con el estado
for s in green:2E7D32:66A86A yellow:F9A825:FBC766 red:C62828:D96A6A; do
  name=${s%%:*}; rest=${s#*:}; body=${rest%%:*}; top=${rest#*:}
  sed -e "s/#1E3A5F; }/#$body; }/" -e "s/\.tapa   { fill: #3E6FA3/.tapa   { fill: #$top/" \
      -e "s/\.iris   { fill: #43A047/.iris   { fill: #FFFFFF/" -e "s/\.pupila { fill: #$body/.pupila { fill: #1E1E1E/" \
      logo.svg > "$T/$name.svg"
  rsvg-convert -w 32 -h 32 "$T/$name.svg" -o "tray/$name.png"
  magick "tray/$name.png" -depth 8 "RGBA:tray/$name.rgba"
done
