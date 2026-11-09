# CAJA standalone

Aplicación de escritorio sin DAW: audio + MIDI del sistema y la misma GUI del plugin.

## Requisitos (Linux)

- Rust estable
- JACK (recomendado) **o** PipeWire/ALSA vía cpal
- Para el Mikro MK3: el dispositivo debe aparecer como MIDI in/out (`aconnect -l`)

```bash
# Dependencias típicas Ubuntu/Debian
sudo apt install build-essential pkg-config libasound2-dev libjack-jackd2-dev
# Opcional JACK server
sudo apt install jackd2
```

## Compilar y ejecutar

Desde la raíz del workspace (`caja_work/`):

```bash
# Debug (más lento, útil para desarrollar)
cargo run -p caja --features standalone

# Release
cargo run -p caja --features standalone --release

# Solo compilar el binario
cargo build -p caja --features standalone --release
# → target/release/caja
```

Ver opciones de audio/MIDI:

```bash
./target/release/caja --help
```

## Mikro MK3

1. Top bar → **MK3 nativo si**
2. Conecta MIDI in/out del Mikro al standalone (argumentos CLI o defaults de JACK)
3. **PATTERN** → elige patrones A1–A16 (Shift = banco B)
4. **STEP** → edita 16 pasos del compás; LEFT/RIGHT cambia compás
5. **PAD_MODE** → pads disparan el kit

Si usas CAJA **dentro de Bitwig con el script**, deja **MK3 nativo no**.

## Ajustes recomendados en la GUI

| Parámetro | Valor |
|-----------|--------|
| Modo | Siempre |
| BPM libre | el tempo que quieras |
| MK3 nativo | Sí |
| Biblioteca | ruta a tus samples |

## Plugin vs standalone

| | Plugin (CLAP/VST3) | Standalone |
|--|--------------------|------------|
| Feature | (ninguna o `vst3`) | `--features standalone` |
| Host | Bitwig, etc. | nih-plug (JACK/cpal) |
| Transporte | del DAW | “Siempre” / tecla |
| MK3 | script Bitwig **o** nativo | nativo |
