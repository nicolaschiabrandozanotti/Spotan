<div align="center">

[English](README.en.md) | **Español**

<img src="docs/img/banner.svg" alt="Spoti" width="720">

**Un cliente de Spotify, YouTube y SoundCloud liviano, en Rust, con letras sincronizadas y fondo animado.**

![Rust](https://img.shields.io/badge/Rust-1.99-orange?logo=rust)
![Licencia MIT](https://img.shields.io/badge/licencia-MIT-green)
![Plataformas](https://img.shields.io/badge/plataformas-Linux%20%7C%20Windows-blue)

</div>

## Qué es

Spoti es un cliente de Spotify liviano hecho en Rust. Parte de un fork de [ncspot](https://github.com/hrkfdn/ncspot) 1.5.1 y usa [librespot](https://github.com/librespot-org/librespot) para reproducir. Para Spotify necesitás una cuenta **Premium**; YouTube y SoundCloud andan sin cuenta, así que también podés usar Spoti sin Spotify.

## Características

- 🪟 **Ventana nativa en Wayland**, dibujada con Cairo y memoria compartida: sin terminal y sin GTK.
- ▶️ **YouTube y SoundCloud** (opcionales, sin cuenta): buscás y escuchás de los tres servicios en la misma lista y en la misma cola. Ver [YouTube y SoundCloud](#youtube-y-soundcloud).
- 🎤 **Letras sincronizadas** desde [LRCLIB](https://lrclib.net), con `Shift+L`. En ventanas angostas, muestra las letras a pantalla completa.
- 🌌 **Fondo animado** de caracteres braille Unicode (se activa o desactiva con `v`).
- 🎨 **Paleta Athena**, leída en vivo desde la sección `[theme]` del `config.toml` de ncspot.
- 🖱️ **El foco sigue al mouse** entre paneles; controles y barra de progreso clickeables.
- 💻 **X11 o terminal**: los mismos paneles dentro de una terminal con `ncspot --panels`.
- 🪟 **Windows**: corre en Windows Terminal con los mismos paneles de letras y fondo (no hay ventana nativa; el foco por hover puede no andar en terminales, el click sí).
- 🧩 **Popover Athena** opcional para Hyprland (`spoti-popover`, con wlr-layer-shell).

## Métricas

<div align="center">
<img src="docs/img/metrics-es.svg" alt="Métricas de Spoti" width="720">
</div>

| Medición | Antes | Ahora |
|---|---|---|
| Ejecutable | 34,8 MiB | 12,9 MiB |
| RAM del reproductor (PSS+SwapPss) | ~31,6 MiB | ~29 MiB (+3–4 MiB con el popover Athena) |
| CPU con la ventana oculta | 2,63 % de un núcleo | 1,06 % |
| Hilos | 18 | 8 |
| Allocations por cuadro de letras | 62 | 0 |

Reproduciendo: ~0,7 % de un núcleo. En pausa: ~0,15 %. Se mantienen **320 kbps** y **gapless**.

> Medido en una sola máquina Linux con la ventana nativa. En Windows no se midió. Los valores varían según el tamaño de la ventana, la biblioteca y la canción.

## Descarga e instalación

Para Spotify necesitás **Premium**; para YouTube y SoundCloud no hace falta cuenta. Todas las versiones están en [Releases](https://github.com/nicolaschiabrandozanotti/Spotan/releases).

### Linux (x86_64) — un comando

```sh
curl -fsSL https://github.com/nicolaschiabrandozanotti/Spotan/releases/latest/download/spoti-linux-x86_64.tar.gz | tar xz && ./spoti-linux-x86_64/install.sh
```

Después abrilo con `spoti` o desde el menú de aplicaciones. La instalación es por usuario, **sin sudo**: copia todo a `~/.local`, agrega una entrada de escritorio e instala la fuente JetBrains Mono Nerd Font. Probado en Ubuntu 22.04/24.04, Debian 12, Fedora y Arch (glibc ≥ 2.34). Solo necesita PulseAudio o PipeWire, cairo y xkbcommon, que vienen con cualquier escritorio. Para desinstalar: `./spoti-linux-x86_64/uninstall.sh`.

### Windows 10/11 (x64) — un comando en PowerShell

```powershell
irm https://github.com/nicolaschiabrandozanotti/Spotan/releases/latest/download/spoti-windows-x86_64.zip -OutFile "$env:TEMP\spoti.zip"; Expand-Archive "$env:TEMP\spoti.zip" "$env:LOCALAPPDATA\Spoti" -Force; & "$env:LOCALAPPDATA\Spoti\spoti-windows-x86_64\spoti.bat"
```

Para volver a abrirlo: `%LOCALAPPDATA%\Spoti\spoti-windows-x86_64\spoti.bat`. También podés bajar el `.zip` desde Releases, descomprimirlo y hacer doble clic en `spoti.bat`; en ese caso SmartScreen puede avisar porque el ejecutable no está firmado (*Más información → Ejecutar de todas formas*). La primera vez copia el tema a `%APPDATA%\ncspot\config.toml` sin pisar una config existente y abre Windows Terminal si lo tenés.

La primera vez, Spoti abre una pantalla de inicio con el botón **Iniciar sesión con Spotify**: se abre el navegador, aceptás y volvés a Spoti, que carga tu música y tu biblioteca. La sesión queda guardada.

¿Sin Spotify? En esa misma pantalla marcá **YouTube** y/o **SoundCloud** y tocá **Entrar sin Spotify**. Después podés iniciar sesión de Spotify cuando quieras desde **Ajustes** (`F4`).

## YouTube y SoundCloud

Vienen **apagados**. Para usarlos:

1. Abrí **Ajustes** (`F4`) y marcá **YouTube** y/o **SoundCloud** en *Otros servicios*. Se guarda al instante, sin reiniciar.
2. Andá a **Buscar** y buscá como siempre. Cada tema muestra de dónde viene: `[S]` Spotify, `[YT]` YouTube, `[SC]` SoundCloud.
3. Con dos o más servicios prendidos aparece la barra **`[1 Spotify] [2 YouTube] [3 SoundCloud]`** arriba de los resultados: las teclas `1`, `2` y `3` (o un clic) muestran u ocultan cada servicio.
4. Reproducí, adelantá, pausá y armá la cola mezclando servicios: pasa sola de uno al siguiente.

Bueno saber:
- **No hace falta cuenta** de YouTube ni de SoundCloud. Spoti decodifica el audio adentro (AAC); no usa yt-dlp, mpv ni navegador.
- Apagados **no consumen nada**: el motor de YouTube/SoundCloud arranca recién con el primer tema de esos servicios (unos 2 MiB más de RAM mientras suenan) y, si los apagás, se cierra al pasar al siguiente tema.
- Todavía no hay inicio de sesión ni biblioteca de YouTube/SoundCloud: solo buscar y escuchar. *Similares* y *Agregar a playlist* son solo para temas de Spotify.
- En SoundCloud, algunos temas son vistas previas de 30 s (los que exigen SoundCloud Go+); los temas con cifrado no se pueden reproducir y se saltean.
- Si YouTube o SoundCloud cambian algo de su lado, puede dejar de andar hasta la próxima versión de Spoti.

## Ajustes y app propia de Spotify (opcional)

Arriba, junto a Biblioteca · Buscar · Cola, está el botón **Ajustes** (o `F4`). Ahí podés cerrar sesión (Spoti se reinicia solo) y, si querés, conectar **tu propia app de desarrollador de Spotify**.

**¿Para qué sirve?** La biblioteca (me gusta y playlists) carga sin nada extra. Pero **buscar** y algunas páginas usan la API oficial de Spotify, que con la app compartida responde casi siempre "demasiados pedidos" (error 429). Con tu propia app, la búsqueda vuelve a andar.

**Paso a paso** (una sola vez, con tu cuenta Premium):

1. Entrá a [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard) e iniciá sesión con tu cuenta de Spotify.
2. Tocá **Create app** y completá:
   - **App name**: `Spoti` (o lo que quieras).
   - **App description**: `Cliente de terminal personal`.
   - **Website**: vacío.
   - **Redirect URIs**: agregá `http://127.0.0.1:8989/ncspot_login` (tocá **Add**), con el puerto: sin puerto Spotify la rechaza como *"not secure"*. En Ajustes, el botón **Copiar Redirect URI** la copia por vos.
   - **Which API/SDKs**: marcá solo **Web API**.
   - Aceptá los términos y tocá **Save**.
3. En tu app, entrá a **Settings** y copiá el **Client ID** (32 caracteres). **No hace falta el Client Secret**: Spoti no lo pide ni lo guarda.
4. En Spoti: **Ajustes** → pegá el Client ID (Ctrl+V o clic derecho) → **Iniciar sesión en la API** → aceptá en el navegador. El estado pasa a *activa (app propia)*.

Notas:
- Spotify limita las apps de desarrollador a **5 usuarios**. Si otra persona quiere usar tu app, agregala en el panel de la app → **User Management** con su nombre y el email de su cuenta de Spotify. Si no, que cree la suya (son 2 minutos).
- Con app propia, la búsqueda trae **10 resultados por página** (límite de Spotify para estas apps); al bajar en la lista se cargan más solos.
- **Volver a la compartida** borra tu Client ID de Spoti y vuelve a la app de siempre.

## Atajos

Son los atajos por defecto de ncspot, más los de Spoti.

| Tecla | Acción |
|---|---|
| `Shift+L` | Letras |
| `Space` / `p` | Pausa / reanudar |
| `n` / `b` | Siguiente / anterior |
| `←` / `→` | Retroceder / avanzar 5 s |
| `+` / `-` | Volumen |
| `↑` / `↓` o `j` / `k` | Desplazar las letras |
| `f` | Seguir la canción actual |
| `[` / `]` | Ajustar el desfase de letras (250 ms) |
| `v` | Fondo braille |
| `q` / `Esc` | Cerrar letras / salir |
| `1` / `2` / `3` | En los resultados de búsqueda: mostrar u ocultar Spotify / YouTube / SoundCloud |
| `?` | Ayuda |

## Compilar desde el código

```sh
packaging/build.sh
```

Necesita Docker. Compila Linux en Debian 12 (para tener una glibc vieja) y Windows con compilación cruzada mediante mingw-w64. Los resultados quedan en `dist/`.

Features de Cargo en `ncspot/`:

- `native`: ventana Wayland (implica `panels`).
- `panels`: letras y fondo dentro de cualquier terminal.

El build de Windows usa `--no-default-features --features panels,rodio_backend,crossterm_backend`.

Para publicar una versión: `git tag v0.1.0 && git push origin v0.1.0`. El workflow `.github/workflows/release.yml` compila y sube los dos paquetes a Releases.

La bitácora técnica del desarrollo está en [docs/NOTAS-TECNICAS.md](docs/NOTAS-TECNICAS.md).

## Agradecimientos

Spoti existe gracias a trabajo ajeno:

- **[ncspot](https://github.com/hrkfdn/ncspot)**, de Henrik Friedrichsen y colaboradores: la base de la que partió este proyecto.
- [librespot](https://github.com/librespot-org/librespot): el motor de reproducción.
- [Symphonia](https://github.com/pdeljanov/Symphonia): decodificación de AAC para YouTube y SoundCloud.
- [cursive](https://github.com/gyscos/cursive) y [ratatui](https://ratatui.rs): interfaz de terminal.
- [Smithay Client Toolkit](https://github.com/Smithay/client-toolkit): cliente Wayland.
- [LRCLIB](https://lrclib.net): letras sincronizadas.
- [JetBrains Mono](https://www.jetbrains.com/lp/mono/) y [Nerd Fonts](https://www.nerdfonts.com) (OFL / MIT).

Spotify es una marca registrada de Spotify AB. Este proyecto no está afiliado a Spotify.

## Licencia

El código propio de Spoti es MIT (ver [LICENSE](LICENSE)). El fork de ncspot en `ncspot/` sigue bajo BSD-2-Clause ([ncspot/LICENSE](ncspot/LICENSE)). Las fuentes incluidas conservan sus licencias (`packaging/fonts`).
