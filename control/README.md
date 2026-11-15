# spoti-control

Modelo y comandos compartidos de Spoti, sin dependencias gráficas, del motor
de audio ni del escritorio. `State`, `Track`, `Snapshot`, `Action` y el cálculo
de posición solo usan Rust estándar, serde y serde_json.

`Snapshot` añade `volume: Option<f64>` a la respuesta JSON de ncspot. El volumen
es una fracción entre 0 y 1. La ausencia del campo indica que el servidor no
ofrece volumen. `Action::Volume` genera `volume <porcentaje>`; buscar genera
`seek <milisegundos>ms` o `seek +/-<segundos>s`. Los botones usan `playpause`,
`previous` y `next`.

El módulo `unix`, condicionado a Unix, recibe la ruta del socket y tiene
timeouts y un límite de tamaño para las respuestas. Cada conexión recibe
primero un snapshot terminado en salto de línea. Al enviar comandos se drena
ese saludo antes de escribir, evitando que el cierre del socket descarte el
comando por datos entrantes sin leer.

Un frontend para Windows puede reutilizar el modelo y los comandos e implementar
un transporte apropiado, por ejemplo named pipes. El transporte Windows no
está implementado en este paquete.
