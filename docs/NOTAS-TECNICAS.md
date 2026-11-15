> Bitácora de desarrollo (notas técnicas internas del proyecto).

# Spoti: Spotify en una ventana nativa Rust

El lanzador normal (`spoti` o Spoti en Super + D) abre `ncspot --native`.
El reproductor, las letras sincronizadas y los dos paneles viven en un único
proceso Rust. La ventana dibuja directamente sus celdas en Wayland mediante
Cairo y buffers de memoria compartida; no inicia Foot, tmux, Kitty ni un
intérprete Python.

Se conservan la biblioteca, búsqueda, cola, controles clickeables, paleta y
animación en braille. Las letras reutilizan exactamente el mismo modelo y
renderizado que `spoti-lyrics`, sin conectarse al reproductor por otro proceso.
El desplegable de Athena es el complemento opcional `spoti-popover`, conectado
al IPC. Se compila por separado desde `athena/`, sin GTK. El protocolo compartido
está en `control/`.

```sh
spoti                     # abrir o recuperar la ventana
spoti-panel --memory      # RAM de todos los componentes, incluyendo swap
ncspot                    # variante TUI en la terminal actual
spoti-panel --launch-terminal  # alternativa anterior Foot/tmux, tras cerrar Spoti
```

Shift + L muestra/oculta las letras fuera de los campos de texto. Al escribir
en la búsqueda o línea de comandos, `L` y las otras letras van al texto.
El foco sigue al mouse entre paneles.
En ventanas angostas se muestra un panel a la vez y Shift + L alterna entre biblioteca
y letras a pantalla completa. `q` o Escape sobre las letras vuelve al reproductor;
`q` sobre el reproductor cierra la aplicación. Cerrar la ventana guarda la cola y
la posición. La ventana también admite pegar texto con Ctrl + Shift + V.

El backend está en `ncspot/src/native_window.rs`; la distribución de los paneles
y su enlace con el modelo están en `ncspot/src/ui/native_panels.rs`. El dibujo
de las letras compartido está en `src/lyrics.rs`. `--native-demo` permite revisar
el renderizador sin abrir ninguna sesión de Spotify ni modificar la reproducción.

## Ajustes de memoria

ncspot usa dos trabajadores asíncronos de red en lugar de uno por CPU. También
limita a uno, por defecto, los trabajadores del runtime interno de librespot;
un valor explícito de `TOKIO_WORKER_THREADS` tiene prioridad. Se configura al
principio de `main`, antes de crear hilos, y solo afecta al entorno del proceso.
En Linux con glibc limita el asignador a dos arenas y devuelve páginas libres
cada quince segundos. No cambia el bitrate, la calidad, la biblioteca ni la caché de audio.

El tema en streaming se guarda en `~/.cache/ncspot/librespot/tmp` y no en `/tmp`:
en Debian `/tmp` es tmpfs y retenía en RAM unos 10 MB por tema (20 MB durante la
precarga gapless), fuera del PSS del proceso. Los restos de un cierre forzado se
borran al arrancar. El tema del panel solo se vuelve a leer si cambia el archivo
de configuración, y se quitó un refresco de 400 ms redundante con los 8 FPS.

La prueba del runtime redujo los hilos estables de 18 a 8 en esta máquina.
Con el mismo binario, tema, fragmento y ventana, las medianas de RAM fueron
29,94 y 29,77 MiB: la diferencia es pequeña y no demuestra un ahorro importante
de RAM. Pasaron pausa/reanudación, búsquedas temporales, siguiente/anterior y
la transición automática al siguiente tema. `runtime-results.md` documenta la
comparación y cómo usar un valor explícito para volver a probar el ajuste anterior.

Las descargas de carátulas comparten ahora `reqwest 0.12.28` con OAuth, usando
TLS, HTTP/2 y soporte de proxies. Se retiraron `reqwest 0.13` y
AWS-LC; las letras mantienen `ureq` con rustls/ring. No se modificaron los modelos
de biblioteca, el motor de audio ni Athena. En la misma prueba de reproducción,
el reproductor pasó de 30,1 a 28,4 MiB y el ejecutable de 15,9 a 14,0 MiB.
`network-results.md` conserva la metodología, la verificación HTTPS/SOCKS5h y
el respaldo. Después de restaurar la ventana pequeña, una lectura dio 30,5 MiB
incluyendo Athena; la geometría y las cachés cambian el consumo habitual.

La medición usa PSS + SwapPss: reparte bibliotecas compartidas y cuenta las
páginas enviadas a swap. Así, minimizar o paginar una ventana no se presenta
como una optimización de memoria. `memory-native.json` y `memory-native.txt` conservan la última prueba de
reproducción de la ventana propia; `spoti-panel --memory` muestra los valores
actuales. La última comparación controlada del reproductor solo dio 30,11 MiB
con música en marcha y ventana oculta, después de unificar TLS y evitar JSON
por cuadro. Athena se mide por separado. La geometría y el uso cambian el
consumo; `tls-json-results.md` conserva las muestras y límites actuales,
y `distribution-results.md` la comparación anterior del perfil de compilación.

La alternativa de terminal conserva Foot 1.21.0 y tmux 3.7c, junto con sus
bibliotecas necesarias, extraídos de paquetes oficiales de Debian en `~/.local/share/spoti-runtime/`. No se reemplazó
ningún paquete ni configuración global. tmux 3.7 ofrece el foco por mouse usado
por esta configuración. Los archivos `.deb` originales están en `packages/`.
La alternativa usa `~/.config/spoti/foot.ini`, `~/.config/spoti/tmux.conf` y un
socket privado en `$XDG_RUNTIME_DIR/spoti/tmux.sock` con permisos 0700.
Estos archivos no se usan al abrir la ventana nativa normal.

## Dibujo y actividad en segundo plano

Las letras conservan el acomodo por ancho y contenido, y el adaptador nativo
pinta directamente sus celdas sin el diff y la copia hacia una tercera grilla de
prueba. El raster braille reutiliza el resultado dentro del mismo instante de
animación; el backend reutiliza historiales y vectores de glifos.

Cuando Wayland todavía no devuelve el callback del último cuadro, se siguen
procesando teclas, callbacks, IPC y audio, pero se difiere el dibujo de Cursive.
Esto reduce el trabajo de una ventana oculta y permite aplicar los cambios
pendientes al volver a mostrarla. La variante de terminal conserva su bucle normal.

En una prueba con la misma canción, ventana de 1322 × 682 y escritorio oculto,
la CPU del reproductor pasó de 2,63 a 1,06 % de un núcleo. La RAM mediana pasó
de 27,92 a 28,98 MiB (PSS + SwapPss): no se demostró ahorro de RAM en esta etapa.
`render-results.md` conserva las pruebas, límites y respaldo.

`cargo run --release --locked --example render-profile` mide el dibujo de letras
sin conectar al reproductor; `-- --canvas` usa el adaptador nativo de celdas.
Los contadores de asignaciones se compilan solamente en los ejemplos y las pruebas de perfilado.

## TLS y estado del panel de letras

Todas las conexiones (librespot, OAuth, API web, carátulas y LRCLIB) usan rustls
con el proveedor ring y los certificados raíz del sistema (TLS 1.2 como mínimo).
No se enlaza OpenSSL ni aws-lc: el ejecutable no depende de la versión de
libssl de cada distro. Frente a OpenSSL del sistema, el reproductor bajó de
26,7 a 21,5 MiB sonando, con la misma CPU. La validación de certificados y
nombres de host sigue activada.

Cuando el compositor deja de pedir cuadros (ventana minimizada o en otro
escritorio) durante 3 s y ya devolvió todos los buffers, la ventana nativa
libera su pool de memoria compartida; el próximo dibujo lo recrea completo.

El panel toma prestados los campos necesarios de la cola y conserva su estado
entre cuadros. No clona el `Playable` completo ni lo convierte a JSON en cada
dibujo; solo actualiza los datos de letras y el estado de reproducción cuando
cambian. La cola se desbloquea antes de dibujar o enviar consultas.

`tls-json-results.md` contiene la comparación de RAM y CPU, las pruebas HTTPS,
el perfil por cuadro y la auditoría de metadatos. Esta última se ejecuta sobre
una copia de los archivos de música y no modifica los modelos ni sus cachés.

## Compilar e instalar

```sh
cd ~/.local/share/spoti-rust
cargo test --locked
cargo build --release --locked
install -m 755 target/release/spoti-lyrics ~/.local/bin/spoti-lyrics.new
install -m 755 target/release/spoti-panel ~/.local/bin/spoti-panel.new
mv ~/.local/bin/spoti-lyrics.new ~/.local/bin/spoti-lyrics
mv ~/.local/bin/spoti-panel.new ~/.local/bin/spoti-panel
```

Se instalan mediante reemplazo atómico para no sobrescribir un ejecutable en uso.
Para actualizar el código de la ventana nativa hay que recompilar ncspot y
volver a abrir la aplicación. El binario `spoti-lyrics` sigue disponible como
alternativa en una terminal.

## Uso

- Shift + L: alternar letras, con ncspot al lado.
- El foco sigue al mouse entre los paneles.
- Espacio/p: pausa; n/b: siguiente/anterior; ←/→: buscar ±5 segundos.
- +/-: volumen; ↑/↓ o j/k: explorar letra; f: seguir el tema.
- [ / ]: ajustar sincronización en pasos de 250 ms; q/Esc: cerrar letras.
- v: activar/desactivar el fondo animado en braille Unicode durante la sesión.
- Alt + ←/→ sigue disponible para cambiar de panel con el teclado.
- La cabecera separa Biblioteca, Buscar y Cola, con la vista activa resaltada.
  Las pestañas de la biblioteca también tienen selección visible y se pueden pulsar.
- Los botones al pie de las letras se pueden pulsar con el mouse. La barra de
  progreso permite buscar con clic o arrastre y la rueda desplaza las letras.
- `spoti-lyrics --check`: comprobar búsqueda y tiempos sin mostrar la letra.

La paleta se lee de `~/.config/ncspot/config.toml` y se actualiza cada segundo.
El panel usa el mismo fondo, colores de selección y barra de estado de ncspot.
Para la variante de terminal, la configuración de Kitty está en `~/.config/kitty/spoti.conf`. El foco por mouse
se aplica solo a la instancia de Kitty de Spoti al abrir el panel.
El perfil `~/.config/kitty/spoti-window.conf` elimina el margen y padding externos
de Spoti. El controlador también aplica esos valores a la ventana existente al
abrir las letras, con el fondo de ncspot y opacidad completa para llenar hasta
los bordes; las demás instancias de Kitty conservan su configuración.

Se reutiliza la caché anterior de `~/.cache/spoti-lyrics/`. Las consultas son
HTTPS nativas con timeout; no se necesitan claves de LRCLIB. Si una canción no
tiene tiempos, se muestra su letra para desplazarse manualmente. Los tiempos
LRC permiten resaltar por línea, no por palabra.

El fondo dibuja ondas y partículas en braille con la misma paleta, a un máximo
de 8 cuadros por segundo. Es una animación decorativa, sin captura de audio.
La línea activa y cada texto conservan un fondo opaco para seguir siendo legibles.

El reproductor también usa el mismo dibujo. Su código está en `ncspot/`, basado
en ncspot 1.5.1; la geometría compartida está en `src/braille.rs`. Se pintan solo
celdas vacías, dejando un margen junto al texto y respetando la selección y las
barras. `v` alterna el fondo en el panel bajo el mouse. En el buscador de ncspot,
`v` sigue escribiendo normalmente.

Para compilar el reproductor personalizado (Rust 1.99.1 o posterior):

```sh
cd ~/.local/share/spoti-rust/ncspot
PKG_CONFIG_PATH="$PWD/../build-support/pkgconfig" RUSTUP_TOOLCHAIN=stable ~/.cargo/bin/cargo test --features native --profile distribution --locked --bin ncspot
PKG_CONFIG_PATH="$PWD/../build-support/pkgconfig" RUSTUP_TOOLCHAIN=stable ~/.cargo/bin/cargo build --features native --profile distribution --locked
install -m 755 target/distribution/ncspot ~/.local/bin/ncspot.new
mv ~/.local/bin/ncspot.new ~/.local/bin/ncspot
```

El backend nativo usa las bibliotecas Cairo y xkbcommon del sistema, con
Smithay Client Toolkit para los protocolos Wayland. No depende de GTK.

El perfil `distribution` usa `opt-level = "s"`, ThinLTO y una unidad de
generación de código. Conserva todas las funciones y ajustes de audio. Quita
símbolos del ejecutable para distribuirlo; esa parte reduce el archivo y no
se contabiliza por sí sola como ahorro de RAM. El perfil `optimized` anterior
sigue disponible para recompilar más rápido durante el desarrollo.

En la comparación del 15/11/2026 el ejecutable pasó de 34,8 a 15,9 MiB. Con
música, después de reiniciar cada versión y esperar al menos veinte segundos,
las medianas de treinta muestras fueron 31,6 MiB del reproductor anterior y
27,9 MiB del perfil nuevo (PSS + SwapPss). Las ventanas tenían distinta
geometría y hubo un cambio de canción durante la muestra anterior: es una
comparación orientativa, no un ahorro fijo garantizado. La prueba independiente
del renderizador, a 1322 × 682, produjo exactamente los mismos píxeles con ambos
binarios; los 80 tests del reproductor pasaron con el perfil nuevo.
`distribution-validation.json` y `distribution-results.md` conservan los
resultados y sus límites. No se cambiaron los hilos, la biblioteca ni los
clientes de red en esta optimización.

El reproductor original queda guardado en `backups/native/ncspot.original`.
`spoti-panel --restart-player`, ejecutado con un canal de control de Kitty,
permite cambiar de compilación guardando la cola y retomando la posición.

## Controles en Athena

La flecha debajo de la cápsula de espacios abre los controles al pasar el mouse.
El panel sigue abierto al entrar en él y se cierra al salir. Incluye título,
artista, progreso con clic/arrastre, anterior/siguiente, saltos de 10 segundos,
pausa/reproducción, volumen y mute que restaura el nivel previo.
El volumen corresponde a ncspot, no al volumen general del sistema.

Es un proceso Rust con Wayland layer-shell y Cairo, sin GTK, terminal ni foco
de teclado. Usa el IPC de ncspot, incluyendo volumen absoluto, y lee Matugen desde
`~/.config/waybar/tokens/colors.css`. Solo la flecha recibe el mouse cuando
está cerrado. Su servicio de usuario arranca con la sesión gráfica:

```sh
systemctl --user restart spoti-popover.service
```

Para compilar el panel con las bibliotecas presentes en esta instalación:

```sh
cd ~/.local/share/spoti-rust
PKG_CONFIG_PATH="$PWD/build-support/pkgconfig" cargo build --release --locked --features popover --bin spoti-popover
install -m 755 target/release/spoti-popover ~/.local/bin/spoti-popover.new
mv ~/.local/bin/spoti-popover.new ~/.local/bin/spoti-popover
systemctl --user restart spoti-popover.service
```

`build-support/` permite compilar con las bibliotecas presentes en esta máquina.
El complemento por separado solo necesita Cairo y Wayland; no necesita las
bibliotecas de desarrollo de GTK. [athena/README.md](athena/README.md) documenta
la compilación independiente, configuración, servicio opcional y paquete de fuentes.

La medición posterior de `memory-athena-after.json` registró 30,8 MiB del
reproductor, 3,6 MiB del complemento cerrado después de usarlo y 34,5 MiB en total
(PSS + SwapPss). El panel GTK anterior sumaba 7,7 MiB. Recién iniciado, antes de
abrirlo, el panel nuevo usó 1,6 MiB; abierto usó 3,8 MiB. El reinicio también puede
cambiar la memoria del reproductor, por lo que su variación no se atribuye al panel.

`athena/src/model.rs` y `control/` no dependen de Wayland. El adaptador actual del
popup y la ventana nativa del reproductor sí lo usan. Una distribución para
Windows requiere implementar el frontend y transporte correspondientes; no se
presenta esta separación como una versión Windows ya terminada.

## Verificación

Los tests cubren LRC, pausas y saltos de reproducción, wrapping Unicode, caché
compatible, cambios de canción, controles, detección de ncspot en Fish y
renderizado a varios tamaños, incluyendo el color de todas las celdas. También
cubren teclado nativo, Unicode, cambio de foco y captura del mouse al arrastrar,
alternancia de paneles y cierre de letras dentro del árbol real de Cursive.
