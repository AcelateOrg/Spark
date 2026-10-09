<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/spark-logo-white.png">
    <img src="docs/assets/spark-logo-dark.png" alt="SPARK" width="520">
  </picture>
</p>

<p align="center">
  <b>Модульный code-first игровой движок, созданный с нуля специально под разработку игр с помощью нейросетей.</b><br>
  Ядро на Rust + wgpu &middot; код игры на Luau &middot; нативные exe, без браузера.
</p>

<p align="center">
  <a href="https://github.com/AcelateOrg/Spark/actions/workflows/ci.yml"><img src="https://github.com/AcelateOrg/Spark/actions/workflows/ci.yml/badge.svg" alt="Build"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MPL--2.0-blue" alt="License: MPL-2.0"></a>
</p>

<p align="center">
  <a href="README.md">English</a> &middot; Русский
</p>

---
> Внимание: SPARK находится в активной разработке. Он имеет множество багов которые могут сломать вашу игру. Мы предоставляем движок "как-есть"

## Зачем

Современные модели уже умеют генерировать рабочие игры: лоу-поли визуал, спрайты, процедурный звук, игровую логику.
Но почти всегда — в связке HTML + JS / Three.js в браузере. А браузер для игр — сплошная боль:

- **Просадки и плохая оптимизация.** Заставить Three.js или Pixi работать без микрофризов на любом железе — ад.
- **Никаких нативных билдов.** Браузер заперт в песочнице: нет нормальных `.exe`, работы с памятью и прямого доступа к железу.
- **Проблемы с дистрибуцией.** Игру на HTML сложно упаковать для Steam или itch.io. Скидываешь билд другу —
  у одного лагает Chromium, у другого ломается рендер в Firefox.

**SPARK** решает это на корню: даёт ИИ нативную быструю среду исполнения с кодовой базой на **Luau** — языке,
который нейросети понимают и генерируют отлично. Главная цель: ИИ-агент сам собирает готовый проект —
от ассетов до геймплея, а ты получаешь нативное окно и один exe, который можно скинуть кому угодно.
Ну и, само собой, писать игры руками никто не запрещает.

SPARK не пытается тягаться с Unity, Unreal Engine или Godot. У него другая задача: быть лучшей платформой
для игр, которые пишет ИИ.

## Сделан под ИИ-агентов

- **Маленький прямолинейный API, один способ сделать вещь.** Весь Luau API — один файл:
  [docs/LUAU_API.md](docs/LUAU_API.md). Отдай его модели.
- **Документация живёт в каждом проекте.** `spark new` кладёт в папку игры `AGENTS.md` и справку по API именно
  этой версии движка — любой агент (Claude Code, Codex, Cursor, ...) сразу знает правила игры.
- **Агент видит, что наделал.** Headless-запуск рендерит кадры в PNG и выгружает сцену текстом
  (`--headless --frames N --screenshot shot.png --dump-scene`) — без окна и без человека.
- **Ошибки сразу говорят, как чинить.** Неизвестные ключи, опции и имена падают с `file:line`, списком допустимых
  значений и подсказкой. Headless-запуск с ошибкой завершается с кодом 1.
- **Горячая перезагрузка.** Сохранил `.luau` или шейдер — игра тут же перезапускается.
- **Code-first.** Никакого GUI-редактора, файлов сцен и скрытого состояния: сцена, логика, UI и ассеты описываются кодом.

## Возможности

| Модуль | Что умеет |
|---|---|
| **Render** | wgpu (Vulkan / DirectX 12 / Metal / запасной OpenGL). 3D-меши и примитивы, модели glTF 2.0 со скелетом и анимациями, солнце / точечные и прожекторные источники света, туман, свои WGSL-шейдеры поверхностей и пост-процессинга с горячей перезагрузкой ([docs/SHADERS.md](docs/SHADERS.md)), низкое внутреннее разрешение для слабых ПК |
| **2D** | immediate-mode отрисовка: спрайты и спрайт-листы, текст любым TTF/OTF-шрифтом, фигуры, трансформации, слои UI и сцены |
| **Script** | рантайм Luau с горячей перезагрузкой, модули (`require`), таймеры, задачи-корутины с `wait`, твины |
| **Input** | полная клавиатура (пунктуация, numpad, F1-F24), ввод текста с IME, автоповтор, мышь с боковыми кнопками и захватом для FPS, геймпады с горячим подключением |
| **Audio** | wav / ogg / mp3 / flac, 3D-звук, фейды, шины микширования (music / sfx / ui ...), многослойная музыка |
| **Events** | события с приоритетами, одноразовые подписчики, события объектов, отложенные события, столкновения |
| **Physics** | rapier3d: динамические / кинематические / статические тела, контроллер персонажа, рейкасты, события столкновений, пикинг по отрисованным мешам |
| **Ship** | `spark build` упаковывает игру в **один exe** (+ zip). Сохранения, полноэкранный режим, иконка окна, сплэш |

## Установка

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.ps1 | iex
```

Скрипт скачает последний релиз в `%LOCALAPPDATA%\Spark\bin` и добавит его в `PATH` (права администратора не нужны).

**Linux / macOS** (терминал):

```sh
curl -fsSL https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.sh | sh
```

Ставит в `~/.local/bin` (другая папка: `SPARK_BIN=/usr/local/bin`) и подскажет, если папки нет в `PATH`.

Открой новый терминал:

```
spark new mygame          # рабочая игра в рекомендуемой структуре
spark mygame              # запуск; правишь любой файл — игра перезагружается
spark check mygame        # найти ошибки в скриптах без окна (CI / ИИ-агенты; --json)
spark build mygame        # dist/mygame(.exe) — один файл, всё внутри
spark update              # потом: поставить свежую версию Spark
```

Или скачай архив для своей системы со страницы [Releases](https://github.com/AcelateOrg/Spark/releases) и положи
`spark` / `spark.exe` куда угодно. Бинарники пока не подписаны: Windows SmartScreen может написать «Неизвестный
издатель»; на macOS скачанному браузером файлу нужен `xattr -d com.apple.quarantine spark` (установщику — нет).

### Платформы

| ОС | Готовый релиз | Графика | Примечания |
|---|---|---|---|
| Windows 10/11 x64 | `spark-windows-x64.zip` | DirectX 12 / Vulkan | сборки для игроков без окна консоли |
| Linux x64 (X11 и Wayland) | `spark-linux-x64.tar.gz` | Vulkan (запасной OpenGL) | нужны ALSA и libudev (есть в десктопных дистрибутивах) |
| macOS 11+ Apple Silicon | `spark-macos-arm64.tar.gz` | Metal | `spark build --app` дополнительно делает `.app` |
| остальные (Linux arm64, Intel Mac, ...) | - | | сборка из исходников |

`spark build` собирает исполняемый файл для той ОС, на которой запущен: `Name.exe` + `Name.zip` на Windows, `Name`
(с флагом исполнения) + `Name.tar.gz` на Linux, `Name` + `Name.zip` (и по желанию `Name.app`) на macOS.

### Сборка из исходников

Нужен [Rust](https://rustup.rs) 1.85+. На Linux ещё dev-пакеты для звука, геймпадов и клавиатуры:

```sh
sudo apt install libasound2-dev libudev-dev libxkbcommon-dev libdbus-1-dev pkg-config   # Debian / Ubuntu
sudo dnf install alsa-lib-devel systemd-devel libxkbcommon-devel dbus-devel             # Fedora
```

```
git clone https://github.com/AcelateOrg/Spark.git
cd Spark
cargo build --release -p spark-cli
target/release/spark new mygame
```

## Быстрый старт

Минимальная игра (`main.luau`):

```lua
local cube

function start()
    spawn(Mesh.plane(20), "green")
    cube = spawn(Mesh.cube(), "orange", { position = vec3(0, 0.5, 0) })
end

function update(dt)
    local speed = if input.down("space") or input.pad_down("a") then 4 else 1
    cube:rotate_y(dt * speed)
end
```

Примеры в репозитории:

```
target/release/spark examples/demo        # 3D: физика, свои шейдеры, звук
target/release/spark examples/starfall    # 2D-игра только на draw.* (клавиатура, мышь или геймпад)
cargo run -p hello                        # то же самое на чистом Rust
```

## Как работать с ИИ-агентом

1. `spark new mygame` и открыть папку в своём агенте.
2. Описать игру. Агент читает `AGENTS.md` и `docs/SPARK_API.md` и пишет код.
3. Он сам проверяет себя: `spark . --headless --frames 120 --screenshot shot.png` — и смотрит на картинку.
4. Ты играешь через `spark .` — каждое сохранение перезагружает игру.
5. `spark build .` — и у тебя один `.exe`, который можно скинуть друзьям или залить на itch.io.

После обновления движка `spark docs mygame` обновит справку по API внутри игры.

## Командная строка

```
spark path/to/game [флаги]          запустить игру (папка с main.luau)
spark new path/to/game               новая игра: main.luau, src/, assets/, game.toml, AGENTS.md, docs/
spark docs path/to/game              обновить docs/ и AGENTS.md
spark build path/to/game [--out DIR] [--loose] [--no-zip]
spark update [VERSION]               поставить последний (или указанный) релиз поверх этого spark.exe
spark --version
```

`spark = "0.1"` в `game.toml` игры — версия движка, под которую она сделана; при другой минорной версии
(до 1.0) движок предупредит и подскажет, что делать.

Флаги любой игры: `--headless`, `--frames N`, `--screenshot PATH`, `--dump-scene`, `--size WxH`, `--fixed-dt S`,
`--no-vsync`, `--splash` / `--no-splash`, `--fullscreen`, `--windowed`, `--help`.
В окне: F12 — скриншот, F11 / Alt+Enter — полный экран. Переменные окружения: `RUST_LOG=debug`, `WGPU_BACKEND=dx12|vulkan|gl`.

## Архитектура

| Крейт | Роль |
|---|---|
| `spark` | фасад: `App`, prelude, реэкспорты — его подключать из Rust |
| `spark-core` | данные мира без GPU: сцена, ассеты, материалы, ввод, время, события, планировщик, очередь звука, виртуальная файловая система |
| `spark-render` | рендер на wgpu: шейдеры поверхностей и пост-процесса, 2D-канва, окно и headless, скриншоты |
| `spark-window` | окно и ввод через winit, геймпады (gilrs) |
| `spark-audio` | звук на kira |
| `spark-physics` | физика на rapier3d |
| `spark-script` | Luau API, модули и горячая перезагрузка (`ScriptGame`) |
| `spark-cli` | исполняемый файл `spark`: run, new, docs, build, update |

Соглашения: Y вверх, правая система координат, 1 единица = 1 метр, объекты смотрят вдоль -Z, радианы (FOV в градусах),
цвета в sRGB.

## Статус

Ранняя версия (0.1). Основная платформа — Windows 10/11 x64; Linux (X11 / Wayland) и macOS собираются и проходят
тесты в CI на каждом коммите (headless), но в реальной игре проверены меньше. API ещё может меняться между версиями — `spark docs` держит документацию
игры в актуальном состоянии.

## Релизы (для мейнтейнеров)

Каждый пуш и pull request запускает [CI](.github/workflows/ci.yml) на Windows, Linux и macOS: clippy (`-D warnings`),
release-сборка, `cargo test`, `spark new` + `spark check --json`. Бинарники — артефакты `spark-windows-x64`,
`spark-linux-x64`, `spark-macos-arm64`.

Релиз: поднять `version` в корневом `Cargo.toml`, запушить и создать релиз на GitHub (Releases > Draft a new release,
тег `v0.1.0`). [Release workflow](.github/workflows/release.yml) соберёт `spark-windows-x64.zip`, `spark-linux-x64.tar.gz`
и `spark-macos-arm64.tar.gz` и прикрепит их к релизу (пара минут); оттуда их качают `install.ps1`, `install.sh`
и `spark update` (имена файлов не менять).

## Лицензия

[Mozilla Public License 2.0](LICENSE). Spark можно использовать в любых проектах, включая закрытые и коммерческие
игры; изменения в файлах самого движка должны оставаться открытыми под MPL. Код и ассеты твоей игры остаются твоими.

Встроенный шрифт: Noto Sans, [SIL Open Font License](crates/spark-core/assets/fonts/OFL.txt).

---

<p align="center">Developed by <b>Acelate</b>, specially for AI &middot; <a href="https://acelate.com/spark">acelate.com/spark</a></p>
