# Athena: complemento opcional de Spoti

`spoti-popover` se compila y ejecuta por separado. No enlaza GTK, GLib, GIO,
el motor de audio ni las letras. No arranca el reproductor. Cerrarlo o desactivar
su servicio deja la música funcionando.

La interacción está en `src/model.rs` y el protocolo en el paquete hermano
`../control`. Estos módulos no dependen del escritorio. `src/wayland.rs` es el
adaptador gráfico para Linux con Wayland y el protocolo wlr-layer-shell
(Hyprland, entre otros compositores compatibles). X11 y Windows necesitan sus
propios adaptadores. La ventana nativa actual de Spoti también usa Wayland;
separar estos paquetes no constituye todavía una versión de Spoti para Windows.

## Compilar el complemento solo

Desde un árbol que contenga los directorios hermanos `athena/` y `control/`:

```sh
cargo test --manifest-path control/Cargo.toml --locked
cargo test --manifest-path athena/Cargo.toml --locked --no-default-features
cargo build --manifest-path athena/Cargo.toml --locked --release --features wayland --bin spoti-popover
```

Requiere Rust 1.85 o posterior y los archivos de desarrollo de Cairo y
pkg-config de la distribución. En Debian: `libcairo2-dev` y `pkg-config`.
La conexión Wayland se carga dinámicamente. No se necesita GTK ni Hyprland
para compilar el modelo, y el complemento no necesita compilar ncspot.
El renderizador sigue los ejemplos MIT de Smithay Client Toolkit 0.19.2.

```sh
install -m 755 athena/target/release/spoti-popover "$HOME/.local/bin/spoti-popover.new"
mv "$HOME/.local/bin/spoti-popover.new" "$HOME/.local/bin/spoti-popover"
```

`systemd/spoti-popover.service` es opcional y puede copiarse a
`$HOME/.config/systemd/user/`. Sin systemd, el ejecutable puede iniciarse desde
el inicio de sesión de cualquier compositor compatible.

## Configuración

```sh
spoti-popover --socket "$XDG_RUNTIME_DIR/ncspot/ncspot.sock" \
  --theme "$HOME/.config/waybar/tokens/colors.css" --top 34 --offset -24
spoti-popover --output HDMI-A-1
```

Las rutas por defecto respetan `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME` y
`SPOTI_SOCKET`; no contienen nombres de usuario. `--offset` se mide desde el
centro de la pantalla. `--output` elige una salida Wayland; por defecto se usa
la primera anunciada. La posición se recalcula cuando cambia su tamaño.
Sin archivo de Matugen se usa la paleta Athena incorporada. `--preview` abre
el panel inmediatamente y permite probarlo con un socket simulado.

El servicio instalado conserva el nombre y la ubicación del ejecutable anterior.
Los controles de volumen necesitan la versión de ncspot de este árbol, que
incluye `volume <porcentaje>` y el campo `volume` en el estado IPC. Con un
reproductor anterior, la barra de volumen queda deshabilitada.

## Consumo

El panel cerrado espera eventos y revisa los colores cada dos segundos;
no consulta al reproductor. Abierto pide el estado cada 500 ms. El progreso se
actualiza aproximadamente ocho veces por segundo y la apertura dura 160 ms. Usa como máximo
dos buffers, reduce su superficie al cerrar y no mantiene una copia de píxeles
para Cairo. No solicita foco de teclado ni reserva espacio en el escritorio.

En esta máquina se midieron alrededor de 1,6 MiB recién iniciado y 3,6 MiB
después de usarlo, frente a 7,7 MiB de la versión GTK. Son PSS + SwapPss;
las cifras cambian con las bibliotecas compartidas y el uso. La medición completa de esta
instalación se conserva en el árbol principal: `memory-athena-after.json`.

## Distribuir los fuentes por separado

```sh
sh athena/pack-source.sh /tmp/spoti-athena-source.tar.gz
```

Incluye únicamente los paquetes `athena` y `control`, sus fuentes, manifiestos,
locks y documentación. No incluye credenciales, configuración personal,
binarios de ncspot ni carpetas de compilación. El protocolo queda disponible
para otro frontend o transporte en Windows sin llevar GTK ni Wayland.

El modelo y el protocolo también se verificaron por compilación cruzada:

```sh
cargo check --manifest-path athena/Cargo.toml --locked --no-default-features --lib --target x86_64-pc-windows-gnu
```

Esta comprobación valida la separación del código compartido; no ejecuta una
interfaz de escritorio en Windows.
