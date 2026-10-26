# MY_CHANGES: доработка Custom Shape Tool (база: v0.15.1)

Эта ветка (`my-custom-shape`) добавляет в PhotoCraft семь вещей: тихий клик и драг за холстом, режим Pixels, ленивую библиотеку фигур, файлы `.csh`, режим «листик» и случайную смену фигуры. Вся логика лежит в новых файлах. В чужих файлах осталось по несколько строк, они перечислены ниже, чтобы при `git rebase` конфликты чинились за минуты.

## Новые файлы (конфликтов с апстримом быть не может)

| Файл | Что внутри |
|---|---|
| `crates/ui-egui/src/custom_shape_ui.rs` (+ `custom_shape_ui_tests.rs`) | завершение драга, режимы Shape/Pixels и Box/Leaf, опции в баре, Randomize, строка Load/Replace/Save Shapes, ленивые миниатюры |
| `crates/engine/src/custom_shape_cmds.rs` (+ `custom_shape_cmds/tests.rs`) | команды `shape.presets.fillPixels`, `shape.presets.placeLeaf` |
| `crates/engine/src/shape_index.rs` (+ `shape_index/tests.rs`) | лёгкий индекс фигур, LRU-кэш путей, `find_shape` |
| `crates/engine/src/shape_csh.rs` (+ `shape_csh/tests.rs`) | источник `.csh`, команды `shape.presets.cshLoad` / `cshSave`, сохранение библиотек в настройках |
| `crates/psd/src/csh.rs` (+ `csh/tests.rs`) | чтение (индекс + разбор одной фигуры) и запись `.csh` |
| `crates/vector/src/warp.rs` (+ `warp/tests.rs`) | `Leaf`, `LeafMap`, `warp_path`, контур листика |
| `crates/vector/src/thin.rs` (+ `thin/tests.rs`) | `flatten`, `subdivide`, `thin`, `simplify`, `simplify_to`: упрощение плотных контуров (заливка, превью, листик) |

## Точки касания в чужих файлах

Порядок: что искать при конфликте после rebase.

| Файл | Место | Что изменено |
|---|---|---|
| `crates/engine/src/shell_commands.rs` | `SHELL_COMMANDS`, после `tool.resetAll` | 8 строк `ui.customShape.*` (режимы Shape/Pixels, Fit Box/Leaf/Zipper, Randomize): так они попадают в Правка → Сочетания клавиш. Ключей по умолчанию нет |
| `crates/ui-egui/src/prefs_ui.rs` | список вкладки «Комбинации клавиш» (условие `tab == 0 && path.is_empty() && def.is_none() …`) | добавлено `&& !custom_shape_ui::handles(id)`: команды без меню и без клавиши по умолчанию иначе скрыты в этой вкладке |
| `crates/ui-egui/src/shortcut_capture_tests.rs` | конец файла | один тест (назначение клавиши команде Custom Shape) |
| `crates/ui-egui/src/menus.rs` | `invoke_unguarded` и `is_enabled` | по 3 строки: вызов `custom_shape_ui::invoke` и `custom_shape_ui::handles` |
| `crates/engine/src/lib.rs` | список `pub mod` | `pub mod custom_shape_cmds;`, `pub mod shape_csh;`, `pub mod shape_index;` (3 строки) |
| `crates/engine/src/commands.rs` | `build()`, после `presets::specs()` | `v.extend(crate::custom_shape_cmds::specs());` (команды `.csh` регистрируются внутри этого `specs()`) |
| `crates/engine/src/presets/mod.rs` | `struct PresetState` и его `Default` | поле `lazy_shapes` + его инициализация (2 места) |
| `crates/engine/src/presets/mod.rs` | `struct Persisted`, `to_json`, `load_presets_json` | ключ `csh_libraries` (4 места, по 1–3 строки): импортированный `.csh` хранится в настройках как путь к файлу (маленькая библиотека без файла, до 2 МБ, как байты) |
| `crates/engine/src/presets/shapes.rs` | `fn lookup` | тело заменено на один вызов `shape_index::find_shape` (раньше клонировал все группы ради одной фигуры) |
| `crates/engine/src/vector_cmds.rs` | `fn paint_coverage` | `pub(crate)` вместо приватной (одно слово) |
| `crates/psd/src/lib.rs` | список `pub mod` | `pub mod csh;` |
| `crates/vector/src/lib.rs` | список `pub mod` | `pub mod thin;`, `pub mod warp;` |
| `crates/ui-egui/src/lib.rs` | список `pub mod` | `pub mod custom_shape_ui;` |
| `crates/ui-egui/src/state.rs` | `struct ToolOptions` и `impl Default` | 11 полей: `custom_shape_default_size`, `custom_shape_mode`, `custom_shape_fit`, `leaf_width`, `leaf_fullness`, `shape_random_every`, `shape_random_source`, `shape_random_set`, `shape_random_seed`, `shape_random` (не сохраняется), и такие же 10 строк в `Default`. Старые настройки грузятся (на структуре уже стоит `#[serde(default)]`) |
| `crates/ui-egui/src/vector_ui.rs` | `finish_shape` | в начале: `if tool == Tool::CustomShape { return custom_shape_ui::finish(...) }`. Старая ветка Custom Shape ниже осталась как есть и больше не выполняется |
| `crates/ui-egui/src/vector_ui.rs` | `fill_param`, `stroke_param` | `pub(crate)` вместо приватных |
| `crates/ui-egui/src/vector_ui.rs` | `shape_preview` | превью «листика» (3 строки до `let path`), `shape_path` вместо `all_groups`, упрощение плотного пути; функция стала `pub(crate)` |
| `crates/ui-egui/src/vector_ui.rs` | `draw_shape_preview` | в начале вызов `custom_shape_ui::draw_dense_preview` (картинка вместо контура для плотных фигур); контур листика вместо контура изогнутой фигуры |
| `crates/ui-egui/src/canvas.rs` | `alt_samples` | в список инструментов добавлен `Tool::CustomShape`: Alt перед нажатием даёт пипетку |
| `crates/ui-egui/src/vector_ui.rs` | `options_bar`, dropdown режима | для Custom Shape вызывается `custom_shape_ui::mode_dropdown`, для остальных инструментов прежний код |
| `crates/ui-egui/src/vector_ui.rs` | `options_bar`, ветка `Tool::CustomShape` | строка `custom_shape_ui::options(app, ui);` после `shape_picker` |
| `crates/ui-egui/src/preset_panels.rs` | `shapes_panel` | список строится из `shape_index::index`, миниатюры только для видимых ячеек, строка `custom_shape_ui::library_row(app, ui);` |
| `crates/ui-egui/src/preset_panels.rs` | `shape_picker` | то же: индекс вместо `all_groups`, ленивые миниатюры |
| `crates/ui-egui/src/preset_panels.rs` | `fn shape_texture` | добавлен `#[allow(dead_code)]` (функция больше не используется) |

Самые частые места конфликтов в апстриме: `options_bar` и `shape_preview` в `vector_ui.rs`, `shapes_panel` / `shape_picker` в `preset_panels.rs`, `ToolOptions` в `state.rs`. Если апстрим перепишет любой из них, достаточно заново вставить вызовы функций из `custom_shape_ui.rs`.

## Панель «Цвет»: смеситель, блокировка яркости, колесо

Новый файл `crates/ui-egui/src/color_mixer_ui.rs` (+ `color_mixer_ui_tests.rs`). Точки касания в чужих файлах:

| Файл | Место | Что изменено |
|---|---|---|
| `crates/ui-egui/src/lib.rs` | список `pub mod` | `pub mod color_mixer_ui;` |
| `crates/ui-egui/src/state.rs` | `ColorPanelState` | поля `mixer_steps`, `lock_brightness`, `wheel` (старые настройки грузятся) |
| `crates/ui-egui/src/panels.rs` | `dock_body`, ветка `(Group::Color, 0) if pro` | строка `crate::color_mixer_ui::panel(app, ui);` после `color_field` |
| `crates/ui-egui/src/panels.rs` | `color_field` | высота поля уменьшена на `MIXER_HEIGHT`; ранний выход в `wheel_field` после чипов; `apply_lock` после выбора оттенка на полоске |
| `crates/ui-egui/src/panels.rs` | `srgb_hsva`, `hsva_srgb`, `hsva_color32` | `pub(crate)` |
| `crates/ui-egui/src/i18n/ru.tsv` | конец | переводы кнопок и команд смесителя |
| `crates/ui-egui/src/state.rs` | `ColorPanelState` | + `mixer_fixed`, `mixer_ends`, `mixer_seen` (замороженная полоска) |
| `crates/engine/src/shell_commands.rs` | после `ui.customShape.*` | 2 строки `ui.colorMixer.next/prev` (хоткеи шага по полоске; клавиш по умолчанию нет) |
| `crates/ui-egui/src/menus.rs` | `invoke_unguarded` и `is_enabled` | вызов `color_mixer_ui::invoke` (3 строки) и `|| color_mixer_ui::handles(id)` |
| `crates/ui-egui/src/prefs_ui.rs` | тот же список «Комбинации клавиш», что у Custom Shape | `&& !color_mixer_ui::handles(id)` |

Смеситель — ряд от основного к фоновому цвету (3–29 шагов, интерполяция по sRGB), клик по плитке ставит цвет. Блокировка яркости держит относительную яркость (Rec.709, линейный свет) при смене оттенка, подбирая V. Колесо — кольцо оттенка и неподвижный треугольник S/V (чистый оттенок справа, белый слева вверху, чёрный слева внизу); квадрат остаётся видом по умолчанию. Кнопка «Менять цвет»: включена — полоска перестраивается по цветам; выключена — полоска заморожена, пока цвет не изменён где-то ещё (круг, hex, пипетка, обмен цветов). Хоткеи шага: Правка → Сочетания клавиш → «Color Mixer».

## Что где работает

- **Клик и драг за холстом** (п. 1–2): `custom_shape_ui::finish`. Клик ничего не открывает; с `custom_shape_default_size ≥ 1` ставит фигуру такого размера. Драг, начатый на серой области, работает. Фигура целиком вне холста пропускается молча, без сообщений.
- **Pixels** (п. 3): опция `custom_shape_mode`; команда `shape.presets.fillPixels` (одна запись в истории; на не пиксельном слое создаёт новый слой в том же шаге).
- **Ленивые библиотеки** (п. 4): `PresetState.lazy_shapes`; встроенные фигуры остаются как были. Панели используют `shape_index::index` (только имена) и разбирают путь, когда он нужен. `shape.presets.list` лениво загруженные библиотеки пока не показывает.
- **`.csh`** (п. 5): панель Shapes, строка кнопок Load / Replace / Save Shapes. Файл читается не целиком: строится индекс имён (148 МБ за 6 мс), а каждая фигура читается с диска, когда нужна. В настройках хранится путь к файлу. **Replace Shapes** убирает и встроенные группы (`shape.presets.reset` их возвращает).
- **Молния (Fit: Zipper)**: как листик, но фигура кладётся в прямоугольник вдоль AB (длина AB, ширина листа в самом широком месте) без деформации и обрезается контуром листика. В векторе это операция «пересечение» (контур листика идёт последним подконтуром). Команды те же: параметр `clip: true` в `leaf`.
- **Хоткеи**: Правка → Сочетания клавиш → искать «Custom Shape». Команды `ui.customShape.mode.shape|pixels|toggle`, `.fit.box|leaf|zipper|cycle`, `.random.toggle`; по умолчанию без клавиш. При нажатии в строке состояния пишется новое значение.
- **Листик** (п. 6): `Fit: Box | Leaf`, ширина и полнота. ⇧ ставит ось AB по 45°, ⌥ делает точку нажатия серединой AB. Работает и в Shape, и в Pixels.
- **Randomize** (п. 7): «Randomize every N» (0 выкл., 1 каждый раз), источник Library / Group / Set. Счётчик живёт в `ToolOptions.shape_random` и не сохраняется. `shape_random_seed` фиксирует последовательность (для тестов).

## Что проверено и что нет

- Формат `.csh` выяснен по настоящим файлам (`Lineart 1-1-*.csh`, 148–193 МБ, по 180 фигур) и проверен на одном из них: индекс, чтение первой и последней фигуры, загрузка по пути, заливка в Pixels. Проверка на реальном файле лежит в двух тестах с `#[ignore]` и переменной `CSH_SAMPLE` (`crates/psd/src/csh/tests.rs`, `crates/engine/src/shape_csh/tests.rs`).
- Запись `.csh` проверена только круговым тестом и сверкой раскладки байтов с настоящими файлами. Откроет ли такой файл Photoshop, не проверялось.
- Векторный слой-фигура (режим Shape) держит не больше 4096 узлов, а фигуры из `Lineart` имеют около 34 000. Поэтому такие фигуры работают только в режиме Pixels; в Shape приложение пишет подсказку. Лимит не менялся: это защита самого движка.
- Визуальная проверка интерфейса скриншотом (`snapshot`) не делалась; панель и режимы смотрелись вручную по скриншоту пользователя.

## Проверка после rebase

```sh
cargo test -p photocraft-psd -p photocraft-vector -p photocraft-engine -p photocraft-ui-egui --lib
cargo clippy -p photocraft-psd -p photocraft-vector -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```
