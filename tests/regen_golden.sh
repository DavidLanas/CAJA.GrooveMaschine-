#!/bin/sh
# Regenera los fixtures de paridad de core/tests/golden/ desde las implementaciones de REFERENCIA:
#   - la appweb original (tools/caja_original.html, ejecutada en Node por tests/parity_node.js)
#   - el motor C++ anterior (tools/cpp_ref/, compilado con g++)
# Requiere: node, g++ (C++17). Se ejecuta desde la raíz del proyecto:  sh tests/regen_golden.sh
# Si algo difiere entre Node y C++ se detiene y lo dice: no se "corrige" nada en silencio.
set -e
G=core/tests/golden
B=$(mktemp -d)
CXX=${CXX:-g++}
(cd tools/cpp_ref && "$CXX" -std=c++17 -O1 -I. -o "$B/parity_cpp"   ../../tests/parity_cpp.cpp   engine.cpp genres.cpp \
                  && "$CXX" -std=c++17 -O1 -I. -o "$B/dump_mid_cpp" ../../tests/dump_mid_cpp.cpp engine.cpp genres.cpp)
mkdir -p "$G"
# 1) 622 casos: 15 géneros x 40 + 22 frases. Referencia = appweb en Node; el C++ debe coincidir.
node tests/parity_node.js cases > "$G/cases15.txt"
node tests/parity_node.js expect < "$G/cases15.txt" > "$G/expected15.txt"
"$B/parity_cpp" < "$G/cases15.txt" | cmp - "$G/expected15.txt" && echo "C++ == appweb en $(wc -l < "$G/cases15.txt") casos"
# 2) 280 casos + 12 frases de los 7 géneros que solo existen en C++. Referencia = C++.
node tests/gen_cases_new7.js > "$G/cases7.txt"
"$B/parity_cpp" < "$G/cases7.txt" > "$G/expected7.txt"
# 3) archivos .mid completos (formato 0 y 1): 30 de la appweb, 44 del C++
rm -rf "$G/mid_node" "$G/mid_cpp"; mkdir -p "$G/mid_cpp"
"$B/dump_mid_cpp" "$G/mid_cpp"
node tests/dump_mid_node.js "$G/mid_node"
for f in "$G"/mid_node/*.mid; do cmp "$f" "$G/mid_cpp/$(basename "$f")"; done && echo "los .mid de Node y C++ coinciden byte a byte"
rm -rf "$B"
echo "Listo. Ahora:  cd core && cargo test --release"
