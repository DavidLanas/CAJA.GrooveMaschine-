# CAJA — Parámetros para macros de Bitwig

Todos estos parámetros aparecen en el dispositivo CAJA dentro de Bitwig
(Remote Controls / Device Remote Controls / MIDI Learn).

## Sección patrón (generación)

| Nombre en host        | ID interno | Rango / notas                          |
|-----------------------|------------|----------------------------------------|
| Genero                | genre      | índice de género                       |
| Compases              | bars       | 0=1, 1=2, 2=4, 3=8 compases            |
| Swing                 | swing      | 0–100 %                                |
| Humanizacion          | human      | 0–100 %                                |
| Densidad              | density    | 0–100 %                                |
| Semilla               | seed       | 0–999999999                            |
| Acento                | accent     | 0–100 %                                |
| Modo                  | runmode    | transporte / siempre / tecla           |
| Thru                  | thru       | on/off                                 |
| Velocidad %           | velscale   | 25–200 %                               |
| CC por paso           | ccnum      | 0–127                                  |
| BPM libre             | bpm        | 60–210                                 |
| Canal MIDI            | channel    | 1–16                                   |
| **Patron activo**     | pattern    | 0–31 = A1–A16, B1–B16 (**nuevo**)      |

## Drum machine global

| Nombre              | ID       | Notas                |
|---------------------|----------|----------------------|
| Solo drum machine   | dmonly   |                      |
| Prioridad al clip   | clipprio |                      |
| Volumen             | master   | −24 … +6 dB          |

## Por pad (P1 … P25)

Para cada pad `n` (1–25):

| Nombre              | ID (base) | Notas                          |
|---------------------|-----------|--------------------------------|
| P{n} Semilla        | padseed   | 0–SEED_MAX                     |
| P{n} Slot           | padslot   | 0–7 (muestra 1–8)              |
| P{n} Mute           | padmute   |                                |
| P{n} Solo           | padsolo   |                                |
| P{n} Pan            | padpan    | −1 … +1                        |
| P{n} Gain           | padgain   | 0 … 2×                         |
| P{n} Guardar slot   | padsave   | trigger (momentary en GUI)     |
| P{n} Ritmo ROLL     | padrate   | golpe / 1c / 2c / 4c           |
| P{n} Modos          | padmode   | bits LOCK/ROLL                 |

En Bitwig aparecen numerados por el array anidado (Pads 1 …).

## Integración / UI

| Nombre              | ID      | Notas                                      |
|---------------------|---------|--------------------------------------------|
| MK3 nativo          | mk3     | Off si usas el script de Bitwig            |
| MK3 LEDs a script   | mk3leds | Escribe `/tmp/caja-leds.json`              |
| Paleta / texto / etc. | palette, textsz, dark, scan | aspecto |

## Cómo mapear en Bitwig

1. Carga CAJA en una pista.
2. Abre el dispositivo → pestaña de **Remote Controls** (o Project Remotes).
3. Pincha el parámetro en la lista del plugin o usa **MIDI Learn** / arrastrar al macro.
4. Para el patrón activo: busca **Patron activo** (A1…B16).

## Flujo recomendado con el script Mikro

- **MK3 nativo = No**
- **MK3 LEDs a script = Sí**
- Script Bitwig: **CAJA LED Bridge = Enabled**
- Los macros de patrón (Swing, Densidad, Patron activo…) los controlas desde Remote Controls o desde el propio script si los asignas.

## Export standalone (ya disponible)

En la GUI del plugin / standalone:

- **Exportar .mid** — patrón como MIDI
- **Exportar .wav** — render de audio del patrón con los samples cargados
- **Abrir carpeta** — carpeta de exports

Ideal: creativo en standalone → export WAV → importar al DAW para mezcla.

## Librería + encoder Mikro (ya disponible)

Modo **Library** (botón SAMPLING del Mikro):

- Encoder: navega subcarpetas
- Encoder press o PLUGIN: entra en la carpeta; si tiene .wav/.flac/.ogg, **carga samples**
- GUI pestaña Librería: misma lista, clic para entrar/cargar

## Ciclo SWING del Mikro (ya con Densidad)

Botón SWING cicla:

1. off (encoder = BPM/Master)
2. Swing
3. Acento
4. Humanize
5. **Densidad**
6. → vuelve a off
