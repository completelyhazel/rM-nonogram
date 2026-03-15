# Nonogram Fetcher para reMarkable Paper Pro

Aplicación para AppLoad que descarga nonogramas de [nonograms.org](https://www.nonograms.org) y los guarda como PDF en tu reMarkable Paper Pro, listos para resolver a mano.

## Características

- Elige entre puzzles en **blanco y negro** o **color**
- Filtra por **tamaño de grid** (5×5 hasta 25×25)
- Filtra por **dificultad** (cualquiera, fácil, medio, difícil)
- El PDF generado incluye:
  - Título del nonograma
  - Pistas de filas y columnas
  - Grid vacío optimizado para rellenar con el lápiz
  - Líneas de referencia cada 5 celdas
  - Crédito y número de puzzle (para verificar la solución en la web)

## Requisitos

### En el reMarkable
- [XOVI](https://github.com/asivery/xovi) instalado
- [AppLoad](https://github.com/asivery/rm-appload) instalado
- [qt-resource-rebuilder](https://github.com/asivery/rmpp-xovi-extensions/tree/master/qt-resource-rebuilder) instalado (dependencia de AppLoad)
- Conexión a internet activa en el reMarkable

### Para compilar
- Rust toolchain (`rustup`)
- Cross-compiler: `gcc-aarch64-linux-gnu` (`apt install gcc-aarch64-linux-gnu`)  
  o la herramienta `cross` (`cargo install cross`)
- Qt Resource Compiler: `rcc` (de `qtbase5-dev-tools` o `qt6-base-dev-tools`)

## Instalación

### 1. Compilar el backend

```bash
cd backend
rustup target add aarch64-unknown-linux-musl
./build.sh
```

Si prefieres usar `cross` (más sencillo, no necesitas el cross-compiler manual):
```bash
cd backend
cargo install cross
cross build --release --target aarch64-unknown-linux-musl
cp target/aarch64-unknown-linux-musl/release/entry ./entry
```

### 2. Empaquetar la app

```bash
cd ..
chmod +x package.sh
./package.sh
```

Esto genera la carpeta `dist/nonogram-fetcher/` con toda la estructura necesaria.

### 3. Copiar al reMarkable

```bash
scp -r dist/nonogram-fetcher root@<IP_DEL_REMARKABLE>:/home/root/xovi/exthome/appload/
```

### 4. Reiniciar AppLoad

En el reMarkable, reinicia el proceso `xochitl` o simplemente reinicia el dispositivo. La app aparecerá en el menú de AppLoad.

## Uso

1. Abre **AppLoad** en el reMarkable
2. Pulsa el icono de **Nonogram Fetcher**
3. Elige el tipo (B&W o Color), el tamaño y la dificultad
4. Pulsa **Fetch Nonogram**
5. Espera unos segundos mientras se descarga y genera el PDF
6. El documento aparecerá directamente en tu biblioteca del reMarkable

## Estructura del proyecto

```
nonogram-fetcher/
├── manifest.json          ← Configuración de la app para AppLoad
├── icon.png               ← Icono (añádelo tú, 96×96 PNG)
├── package.sh             ← Script para empaquetar
├── ui/
│   ├── main.qml           ← Interfaz de usuario (QML)
│   └── application.qrc    ← Recursos QML
└── backend/
    ├── Cargo.toml
    ├── build.sh            ← Script de cross-compilación
    ├── .cargo/config.toml  ← Configuración del linker
    └── src/
        ├── main.rs         ← Punto de entrada + loop de mensajes
        ├── appload.rs      ← Protocolo IPC con AppLoad
        ├── nonogram.rs     ← Scraper de nonograms.org
        └── pdf_gen.rs      ← Generación del PDF con printpdf
```

## Notas técnicas

### Protocolo AppLoad (IPC)
El backend se comunica con el frontend QML a través de un Unix socket. Cada mensaje tiene el formato:
- 4 bytes en little-endian indicando la longitud N del JSON
- N bytes de JSON UTF-8 con estructura `{ "type": u32, "contents": "..." }`

| Tipo | Dirección | Significado |
|------|-----------|-------------|
| 0 | Frontend → Backend | Petición de fetch (JSON con parámetros) |
| 1 | Backend → Frontend | Éxito (`SAVED:<ruta>`) |
| 2 | Backend → Frontend | Error (mensaje descriptivo) |
| 3 | Backend → Frontend | Progreso (texto de estado) |

### Scraping de nonograms.org
Los datos del puzzle están embebidos en la página HTML como variables JavaScript:
- `var d=[[...]]` → solución/grid (usado para calcular las pistas)
- `var s={...}` → metadatos, incluyendo colores para puzzles a color

### PDF generado
El PDF se guarda en `/home/root/.local/share/remarkable/xochitl/` con nombre `nonogram_<id>_<titulo>.pdf`. xochitl lo detecta automáticamente y lo añade a la biblioteca.

## Problemas conocidos

- La heurística de filtrado por dificultad es aproximada (basada en el ID del puzzle)
- Algunos puzzles muy grandes (25×25 con muchas pistas) pueden quedar con texto pequeño
- Se requiere conexión a internet; si el reMarkable no tiene WiFi activo la app mostrará un error

## Licencia

GPL-3.0 — mismo que XOVI y AppLoad.
