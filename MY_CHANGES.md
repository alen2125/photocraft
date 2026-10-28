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
| `crates/ui-egui/src/canvas.rs` | `sample_eyedropper` | строка `crate::color_mixer_ui::eyedropper_sampled(app);` после `tools.setColors` (кнопка «Пипетка» заморозки полоски) |
| `crates/ui-egui/src/panel_docking.rs` | `body`, `ScrollArea` | то же для нового дока: `fixed` для pro-вкладки «Цвет», прокрутка и полоса выключены |
| `crates/ui-egui/src/dock.rs` | `ScrollArea` вкладки (ветка не `scrolls_itself`) | `fixed` для pro-вкладки «Цвет»: прокрутка и полоса выключены (`.scroll([false, !fixed])`, `AlwaysHidden`) |
| `crates/ui-egui/src/panels.rs` | `dock_body`, pro «Цвет» | кнопка HSB/HSL убрана из конца вкладки (её рисует `color_mixer_ui::panel` в ряду кнопок); `remember_height` измеряет высоту под полем; `color_slider_mode_row` стала `pub(crate)` |
| `crates/ui-egui/src/menu_keys.rs` | `ALT_MENUS`, `show_underlines`, `keys` | Alt (тап и Alt+буква) больше не фокусирует меню и не подчёркивает буквы; `const ALT_MENUS: bool = cfg!(test)`, чтобы апстримные тесты остались как были. F10 работает |

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

## Доработки: U, ширина листика, Erase, память панели «Цвет», режимы лассо

Новые файлы: `crates/ui-egui/src/tool_default_ui.rs`, `color_panel_memory.rs`, `lasso_modes_ui.rs` (+ `lasso_modes_ui_tests.rs`).

| Файл | Место | Что изменено |
|---|---|---|
| `crates/ui-egui/src/shortcuts.rs` | `handle`, выбор инструмента по клавише | 2 строки: `crate::tool_default_ui::pick` (U не циклит, по умолчанию Custom Shape, иначе последний выбранный в группе) |
| `crates/ui-egui/src/lib.rs` | `pub mod`, кадровый цикл | `pub mod tool_default_ui/color_panel_memory/lasso_modes_ui;` и `color_panel_memory::persist(self, &ctx);` после `brush_picker::persist` |
| `crates/ui-egui/src/prefs_ui.rs` | `load` | `crate::color_panel_memory::restore(app);` после `brush_picker::restore`; тест `color_panel_settings_survive_a_restart` |
| `crates/engine/src/prefs.rs` | `struct Preferences` | поле `color_panel: Value` |
| `crates/ui-egui/src/color_mixer_ui.rs` | `MIN_STEPS`, `MAX_STEPS` | `pub(crate)` |
| `crates/ui-egui/src/state.rs` | `ToolOptions` и `Default` | `leaf_fixed`, `leaf_width_px`, `custom_shape_erase`, `lasso_mode`, `lasso_cell_px` |
| `crates/vector/src/warp.rs` | `MAX_WIDTH` | 4.0 → 1000.0 (фиксированная ширина в px на коротком драге) |
| `crates/engine/src/shell_commands.rs` | после `ui.customShape.*` | `ui.customShape.erase.toggle` |
| `crates/ui-egui/src/canvas.rs` | `finish_gesture` (ветка Lasso), `commit_polygon` | режим выделения через `lasso_modes_ui::mode`, после `select.lasso` вызов `lasso_modes_ui::after_select` |
| `crates/ui-egui/src/magnetic_lasso_ui.rs` | `commit` | то же для `select.magneticLasso` |
| `crates/ui-egui/src/panels.rs` | options bar лассо | строка `crate::lasso_modes_ui::options(app, ui);` после Anti-alias |
| `crates/ui-egui/src/tool_reset.rs` | `Tool::Lasso`/`MagneticLasso` | `lasso_mode, lasso_cell_px` в `reset!` |

В новых местах своих файлов: `custom_shape_ui.rs` (Fixed width, Erase, `sequence` для случайных фигур сетки), `custom_shape_cmds.rs` (параметр `erase` у `fillPixels`, команда `shape.presets.fillGrid`).

Что делают: **U** всегда Custom Shape (или последний выбранный в панели из группы), без цикла. **Fixed width** (Leaf/Zipper): ширина в px не зависит от длины драга. **Erase** (Pixels): фигура стирает альфу. **Панель «Цвет»** помнит настройки между запусками. **Лассо → Mode**: Clear удаляет пиксели внутри, Custom Shape заливает сеткой фигур (ряды слева направо и сверху вниз, ячейка `cell × cell`, фигуры основным цветом, пустоты фоновым, Randomize учитывается). В обоих режимах выделение заменяется и после действия снимается.

## 3D-редактор поз (этап 1: сцена, примитивы, свет, камера, рендер в слой)

Окно **Окно › 3D-редактор поз** (`ui.scene3d.open`): сцена из примитивов (куб, сфера, цилиндр, конус, плоскость), света (солнце, точечный, прожектор) и пустых узлов с иерархией. Камера крутится мышью (ЛКМ орбита, ПКМ/СКМ сдвиг, колесо зум), клик выбирает объект. «Рендер в слой» кладёт картинку в растровый слой документа одним шагом отмены (прозрачное небо по умолчанию). Сцена сохраняется в `.pcscene` (JSON) и открывается обратно. План всего проекта: загрузка glTF/VRM, позы по костям, физика (rapier), пролёт камерой, модели из VaM. Это этапы 2–5, их ещё нет.

Новые крейты (зарегистрированы в `xtask/src/layers.rs`: `scene3d` = L2, `render3d` = L3):

| Крейт / файл | Что внутри |
|---|---|
| `crates/scene3d` (+ `src/tests.rs`) | модель сцены: узлы, иерархия без циклов, примитивы и их меши, материалы, света, камера, формат `.pcscene` с проверкой версии и лимитов (4096 узлов, 64 МБ) |
| `crates/render3d` (+ `src/tests.rs`) | программный рендер (z-буфер, перспективные нормали, Blinn-Phong, до 8 светов, сглаживание, прозрачный фон), выбор объекта лучом `pick`, проекция `project`. Без GPU, работает в CLI и wasm. Теней пока нет |
| `crates/engine/src/scene3d_cmds.rs` (+ `scene3d_cmds/tests.rs`) | команды `scene3d.renderToLayer`, `scene3d.save`, `scene3d.open` (сцена приходит параметром `scene` или `path`, сессия её не хранит) |
| `crates/ui-egui/src/scene3d_ui.rs` (+ `scene3d_ui_tests.rs`) | окно редактора, вьюпорт, инспектор, список узлов, сохранение и открытие сцены |

Точки касания в чужих файлах:

| Файл | Место | Что изменено |
|---|---|---|
| `Cargo.toml` | `[workspace.dependencies]` | `photocraft-scene3d`, `photocraft-render3d` (2 строки) |
| `Cargo.lock` | | появился `glam 0.30` (зависимость `scene3d`) |
| `xtask/src/layers.rs` | `TABLE` | `("scene3d", Class::Layer(2))`, `("render3d", Class::Layer(3))` |
| `crates/engine/Cargo.toml` | `[dependencies]` | 2 строки: `photocraft-scene3d`, `photocraft-render3d` |
| `crates/engine/src/lib.rs` | список `pub mod` | `pub mod scene3d_cmds;` |
| `crates/engine/src/commands.rs` | `build()`, после `render_cmds::specs()` | `v.extend(crate::scene3d_cmds::specs());` |
| `crates/engine/src/shell_commands.rs` | `SHELL_COMMANDS`, перед `window.panel.group` | `ui.scene3d.open` (меню Окно) |
| `crates/ui-egui/Cargo.toml` | `[dependencies]` | 2 строки: те же крейты |
| `crates/ui-egui/src/lib.rs` | `pub mod`, поле `scene3d` в `PhotocraftApp`, `new`, кадровый цикл | `pub mod scene3d_ui;`, поле + `scene3d: Default::default()`, `scene3d_ui::show(self, &ctx);` после `distort_ui::show` |
| `crates/ui-egui/src/menus.rs` | `invoke_unguarded`, `is_enabled` | вызов `scene3d_ui::invoke` (3 строки) и `|| scene3d_ui::handles(id)` |
| `crates/ui-egui/src/i18n/ru.tsv` | конец файла | блок «3D Pose Editor (scene3d_ui)»: 22 строки перевода |
| `.gitignore` | | `/samples/` (тестовые модели не коммитим) |

Проверка:

```sh
cargo test -p photocraft-scene3d -p photocraft-render3d
cargo test -p photocraft-engine --lib scene3d_cmds
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo test -p photocraft-ui-egui --lib scene3d
cargo clippy -p photocraft-scene3d -p photocraft-render3d -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```

Известные ограничения этапа 1: рендер программный (wgpu-ускорение позже); теней нет; цвет рендера считается как sRGB и переводится в формат документа без учёта его ICC-профиля; диалоги «Открыть/Сохранить сцену» проверены только в тестах, не руками.


## 3D-редактор поз (этап 2: модели, кости, позы)

К этапу 1 добавлено: **Импорт модели…** (`.glb`, `.gltf`, `.vrm`, VRM 0.x и 1.0), скелет поверх вьюпорта, поза костей мышью, позы с именами, лимиты суставов. Перетаскивание точки кости: режим **Перемещение (IK)** тянет кость к указателю, поворачивая до N костей выше неё (длины костей не меняются; верхнюю кость, бёдра, двигает целиком); режим **Поворот** крутит кость вокруг осей камеры. Клик по точке выбирает кость, клик по модели выбирает её меш. **Сохранить позу** запоминает повороты и положения всех костей, **Применить** возвращает их, **Сбросить позу** возвращает исходное положение модели. Сцена хранит только путь к файлу модели (геометрия в `.pcscene` не пишется): при «Открыть сцену» модель подгружается заново по пути. Рендер: скиннинг по костям, текстуры (билинейно), альфа-тест (волосы, ресницы), `unlit`-материалы VRM рисуются освещёнными, а их `emissive = 1` игнорируется.

| Крейт / файл | Что внутри |
|---|---|
| `crates/model3d` (L0, `Class::Standalone`, +`src/tests.rs`, `src/testgen.rs`) | чтение glTF/GLB/VRM: узлы, скиннинг, материалы, текстуры, карта костей humanoid, пружинные кости VRM 0.x (волосы, юбка: для этапа 3). Лимиты: файл 512 МБ, 4096 узлов, 4 млн вершин, текстуры до 4096 px и 768 МБ суммарно |
| `crates/scene3d/src/model.rs`, `ik.rs` | `Scene::add_model` (кости и меши как узлы, VRM 0.x разворачивается на 180°), `world_matrices`, позы, FK `rotate_world`, IK `solve_ik` (CCD с лимитами), `translate_to` |
| `crates/render3d` | скиннинг, текстуры, альфа-тест, `project_points`, `pixel_ray` |
| `crates/engine/src/scene3d_cmds.rs` | `scene3d.addModel`; `scene3d.renderToLayer` сам подгружает модели по путям (кэш на 2 модели по хэшу файла) |
| `crates/ui-egui/src/scene3d_ui.rs` (+ `scene3d_ui_models_tests.rs`) | импорт, скелет, перетаскивание, панель «Поза» |

Новые точки касания в чужих файлах (к списку этапа 1):

| Файл | Место | Что изменено |
|---|---|---|
| `Cargo.toml` | `[workspace.dependencies]` | `photocraft-model3d` (1 строка) |
| `xtask/src/layers.rs` | `TABLE` | `("model3d", Class::Standalone)` |
| `crates/engine/Cargo.toml`, `crates/ui-egui/Cargo.toml` | `[dev-dependencies]` | `photocraft-model3d` с `testgen` (тестовая модель) |
| `crates/scene3d/Cargo.toml` | `[dependencies]` | `photocraft-model3d` |
| `crates/ui-egui/src/i18n/ru.tsv` | конец файла | ещё 13 строк перевода |

Проверка (вместе с командами этапа 1):

```sh
cargo test -p photocraft-model3d -p photocraft-scene3d -p photocraft-render3d
cargo test -p photocraft-engine --lib scene3d_cmds
cargo test -p photocraft-ui-egui --lib scene3d
cargo clippy -p photocraft-model3d -p photocraft-scene3d -p photocraft-render3d -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```

Тесты на настоящем VRM (`model3d`, `scene3d::tests_real`) читают `samples/8590256991748008892.vrm` и без него молча пропускаются: `samples/` в `.gitignore`.

Ограничения этапа 2: у суставов нет готовых анатомических лимитов (оси костей у разных моделей разные), лимиты задаёт пользователь в инспекторе кости; материалы VRM рисуются как обычные освещённые (MToon не воспроизводится); полупрозрачность считается как «вырезать ниже порога», без смешивания; пружинные кости VRM только читаются, не симулируются (этап 3); VRM 1.0 читается для humanoid и пружин, но вживую не проверялся (нет образца).


## 3D-редактор поз (раунд 3: режим на холсте, скорость, гизмо)

**Режим на холсте.** «Окно › 3D-редактор поз» больше не открывает отдельное окно с картинкой: сцена рисуется поверх документа (как Puppet Warp), указатель работает в координатах документа. Рендер идёт на весь документ, поэтому то, что видно на холсте, и запекается: **Enter** или «Запечь в слой» создаёт растровый слой размером с документ в точке (0, 0) одним шагом отмены (`scene3d.renderToLayer`), **Esc** закрывает режим без слоя (сцена остаётся в редакторе). Смена или закрытие документа закрывает режим. Справа плавающая панель: список узлов, позы, инспектор, Открыть/Сохранить сцену. Камера: перетаскивание по пустому месту (или Alt), Shift сдвигает, Ctrl приближает; колесо и панорама холста работают как раньше.

**Гизмо.** Перемещение (W), поворот (E), масштаб (R) у выбранного объекта и кости; размер постоянный на экране; «Локально» включает оси узла. Кость у кости перетаскивается через IK, верхняя кость скользит, поворот крутит кость; масштаба у костей нет. Поля Положение/Поворот/Масштаб убраны под «Точные значения». **Ctrl+Z / Ctrl+Shift+Z** отменяют и возвращают правки сцены (камера в историю не входит; перетаскивание = один шаг).

**Скорость.** Замер на вашем VRM (29 тыс. треугольников, 16 потоков, release): кадр 960×600 без сглаживания 53 мс → 19 мс, со сглаживанием 118 мс → 37 мс, итоговый 1024×768 ×2 159 мс → 53 мс. Растеризация идёт параллельными полосами (`rayon`, на wasm по очереди; картинка не зависит от числа потоков, это проверяется тестом), подготовка мешей тоже параллельна. Превью рисуется в фоновом потоке по принципу «последний запрос побеждает» (UI не ждёт), пока камера или поза меняются, это четверть пикселей без сглаживания, через 0,15 с покоя полное качество.

| Файл | Что |
|---|---|
| `crates/scene3d/src/gizmo.rs` (+`tests_gizmo.rs`) | математика гизмо: выбор ручки лучом, перемещение/поворот/масштаб при перетаскивании |
| `crates/ui-egui/src/scene3d_canvas.rs` (+`scene3d_canvas_tests.rs`, `scene3d_canvas_gizmo_tests.rs`) | режим на холсте: указатель, клавиши, гизмо, история, запекание, наложение |
| `crates/ui-egui/src/scene3d_worker.rs` | фоновый рендер превью |
| `crates/render3d/Cargo.toml`, `src/raster.rs`, `src/lib.rs` | `rayon` (только не wasm), параллельные полосы и подготовка |

Точки касания в чужих файлах (к списку раньше):

| Файл | Место | Что изменено |
|---|---|---|
| `crates/ui-egui/src/distort_ui.rs` | `pointer`, `keys`, `draw_content`, `draw_overlay`, `options_bar` | по 3-5 строк в начале каждой: `if app.scene3d.open { crate::scene3d_canvas::...; }` |
| `crates/ui-egui/src/lib.rs` | `pub mod` | `mod scene3d_canvas;`, `mod scene3d_worker;` |
| `crates/ui-egui/src/i18n/ru.tsv` | конец файла | ещё 4 строки перевода |

Проверка: те же команды, что выше; плюс `cargo test -p photocraft-scene3d` (гизмо) и `cargo test -p photocraft-render3d` (параллельность).

Что ещё не сделано: физика, гравитация и привязка узлов (этап 3), волосы, VaM. Гизмо и режим на холсте проверены тестами и снимками, но не мышью вживую.


## 3D-редактор поз (связи узлов, крупные цели)

**Связи** (по мотивам плагина VaM): узел можно привязать к другому («Связать с…» в инспекторе): расстояние, орбита (угол вокруг вертикали), угол над горизонтом, «Целиться на цель» (ось −Z узла смотрит на цель: прожектор и солнце светят на эту точку) и «Поворачиваться вместе с целью» (орбита меряется по осям цели, свет остаётся перед лицом при повороте головы). Цель может быть любым узлом, в том числе костью. Выбор цели ничего не сдвигает: расстояние и углы считаются из текущего положения; «(нет)» оставляет узел там, где он стоит. Двигается цель, вместе с ней едет связанный узел. Гизмо связанного узла правит саму связь: **Перемещение** перенацеливает (меняет расстояние и углы), **Поворот** вращает по орбите вокруг цели, **Масштаб** приближает и отдаляет. На холсте связь рисуется пунктиром с ромбом на цели. Связи сохраняются в `.pcscene`; связь с несуществующим узлом при загрузке и при удалении цели отбрасывается; петли (A→B→A, в том числе через родителей) отклоняются.

**Крупные цели:** зона захвата ручек гизмо вдвое шире, линии гизмо толще (4–4,5 px вместо 2–2,5), ручки на концах 16 px вместо 10, точки костей 11 px вместо 6, захват точки кости 16 px вместо 10.

| Файл | Что |
|---|---|
| `crates/scene3d/src/link.rs` (+`tests_links.rs`) | `Link`, `LinkSpace`, математика орбиты и наведения, `Scene::link_to/unlink/set_link/set_link_from_world/orbit_link/scale_link_distance/depends_on` |
| `crates/scene3d/src/scene.rs`, `model.rs` | поле `Node.link`; `world_matrix` и `world_matrices` считают через общий `link::resolve` (родители и цели связей, с ограничением глубины) |
| `crates/render3d/src/tests_links.rs` | прожектор, связанный с узлом, освещает его; цель двигается, пятно света за ней |
| `crates/ui-egui/src/scene3d_ui.rs`, `scene3d_canvas.rs` (+`scene3d_canvas_link_tests.rs`) | раздел «Связать с» в инспекторе, гизмо связанного узла, пунктир |
| `crates/ui-egui/src/i18n/ru.tsv` | ещё 6 строк перевода |

Новых точек касания в чужих файлах нет (кроме строк перевода).


## 3D-редактор поз (физика, часть 1: гравитация объектов)

У каждого твёрдого объекта (куб, сфера, цилиндр, конус, плоскость) есть галочка **«Привязан (стоит на месте)»**, по умолчанию включена. В списке узлов непривязанные помечены «↓». Кнопка **«▶ Пуск»** запускает физику (rapier, шаг 1/60 с, без случайности): непривязанные объекты падают под гравитацией и сталкиваются друг с другом, с привязанными объектами и с полом (плоскость работает как пол, её верх = поверхность). **«■ Стоп (оставить)»** оставляет результат одним шагом отмены, **«↺ Откатить»** возвращает сцену как была до пуска (камера остаётся). Во время пуска указатель крутит камеру и выбирает, но не берёт гизмо. Размеры и повороты берутся из мировой матрицы узла (родители учитываются); невидимые и вырожденные объекты пропускаются; предел 256 падающих и 1024 всего. Флаг сохраняется в `.pcscene` (по умолчанию привязан, старые файлы не меняются).

**Не сделано (часть 2):** физика костей (ragdoll, руки не проходят сквозь тело), волосы и юбка VRM по пружинным цепочкам (они уже читаются из файла), связанные узлы и меши модели в физике не участвуют.

| Файл | Что |
|---|---|
| `crates/physics3d` (новый крейт L3, `rapier3d`; +`src/tests.rs`) | `World::new(&Scene)`, `step`, `poses`, `apply`, `settled` |
| `crates/scene3d/src/scene.rs`, `link.rs` | поле `Node.pinned`; `Scene::place_world` (записать мировое положение узла в его локальное) |
| `crates/ui-egui/src/scene3d_canvas.rs` (+`scene3d_canvas_physics_tests.rs`), `scene3d_ui.rs` | `play`/`stop`/`tick`, кнопки, галочка, метка в списке |

Точки касания в чужих файлах: `Cargo.toml` (`photocraft-physics3d` в `[workspace.dependencies]`), `xtask/src/layers.rs` (`("physics3d", Class::Layer(3))`), `crates/ui-egui/Cargo.toml` (зависимость), `crates/ui-egui/src/i18n/ru.tsv` (ещё 6 строк). Проверка: `cargo test -p photocraft-physics3d`, плюс прежние команды.


## 3D-редактор поз (физика, часть 2: кости и волосы)

**Кости (ragdoll).** У каждой кости humanoid VRM есть капсула столкновений (размер и группа по имени кости: торс, плечи и бёдра, предплечья/голени/кисти/пальцы). Привязанные кости неподвижны в позе и служат препятствием: предметы ложатся на персонажа, конечности друг в друга и в тело не проходят. Галочка «Привязан» теперь есть и у костей: **отвязанная кость отпускает всю конечность ниже неё**, и по «▶ Пуск» она повисает на суставе выше (шаровой шарнир с лимитом ±1,6 рад по каждой оси, затухание). Кость поворачивается, не растягиваясь; кость без родителя (бёдра) ещё и перемещается. Плечо и бедро не сталкиваются с торсом (капсулы пересекаются в суставе), предплечья, голени и кисти сталкиваются со всем.

**Волосы.** Пружинные кости VRM (волосы, юбка, аксессуары: `secondaryAnimation`, уже читались из файла) качаются при «▶ Пуск»: инерция, гравитация, сопротивление, жёсткость и сферы-коллайдеры из файла (метод Верле, по спецификации VRM). Работают и при полностью привязанном теле: «Пуск» разрешён, если есть падающее или качающееся. На вашей модели найдено больше 20 цепочек.

**Ограничения:** анатомических лимитов суставов нет (оси костей у моделей разные): рука, отпущенная из Т-позы, может уйти за спину; руки и ноги из капсул (не по сетке тела); имя кости ищется по имени узла (две копии одной модели в сцене путаются); меши и связанные узлы не участвуют; VRM 1.0 пружины не читаются. Физика идёт в основном потоке UI (до 6 шагов за кадр).

| Файл | Что |
|---|---|
| `crates/physics3d/src/lib.rs`, `springs.rs` (+`tests_ragdoll.rs`, `tests_real.rs`) | `World`: твёрдые тела, ragdoll, `step_scene`, `is_inert`; пружинные цепочки |
| `crates/scene3d/src/scene.rs`, `model.rs`, `ik.rs` | поле `Node.humanoid` (имя humanoid-кости, ставится при импорте); `set_world_rotation` стала публичной |
| `crates/ui-egui/src/scene3d_canvas.rs`, `scene3d_ui.rs` | `tick` через `step_scene`, галочка и подсказка у костей, метка «↓» |

Новых точек касания в чужих файлах нет (кроме строк перевода в `ru.tsv`, ещё 3).


## 3D-редактор поз (база VaM: этап A, фигуры и текстуры)

**Что.** Скрипт `tools/vam_export/export.py` (Python + UnityPy, ставится в отдельный venv вне репо) достаёт из **файлов самой игры** VaM (`VaM_Data/StreamingAssets`: `f_1`, `m_1`, `a_per`, `*_mat`) базовые женскую и мужскую фигуру Genesis 2 и пишет `samples/vam_base_{female,male}.vrm`: ~25 тыс. вершин, UV, нормали, 80/84 кости с весами, скелет DAZ, humanoid-карточка VRM 0.x (55 костей: физика и IK работают по именам), материалы по слотам (Head, Torso, Hands, Legs, Eyes, Teeth, Eyelashes…) с картами цвета (2048 px, JPEG) и маской ресниц. Роговица, слёзы и блик глаза не рисуются. Запуск: `python tools/vam_export/export.py female|male <папка> [скин, например f_3]`.

**Чего ещё нет:** нормаль-карт и спекуляра (растеризатор их не использует), переключателя скинов в панели, морфов, лимитов суставов из игры, волос и одежды (план: этапы B–F).

**Важно.** Сами модели и выгрузки в git не попадают (`/samples/` в `.gitignore`); в репозитории только скрипт. Читает только архивы игры; `.var`/zip не трогает.


## 3D-редактор поз (база VaM: исправления скиннинга и лишние кости)

**Скиннинг.** Текстура «рвалась» на сгибах из-за трёх ошибок в экспортёре (`tools/vam_export/export.py`): (1) список `fullyWeightedVertices` в VaM это *основная кость* каждой вершины, а не «вес 1.0», его нельзя добавлять к сглаженным весам (отсюда ступеньки на локтях); он используется только для вершин без общих весов; (2) веса индексируются по UV-вершинам (у швов своя запись); (3) в префабе два набора костей (основной на 88 с родителями и вспомогательный на 20): берётся основной, иначе у части костей терялся родитель (линии от локтей к ногам).

**Лишние кости.** В VaM руками двигаются ~25 контроллеров (hip, pelvis, abdomen, abdomen2, chest, neck, head, плечи, руки, бёдра, колени, стопы, пальцы стоп); пальцы, язык и лицо там не кости, а «pose control» ползунки (набор морфов, вращающих кости на заданный угол). Поэтому: у модели есть список `controls` (расширение `PC_controls` в glTF, читает `crates/model3d/src/vrm.rs`), остальные суставы получают флаг `Node.minor`: без точки на холсте и строки в списке, но их можно поворачивать из кода, они сохраняются в позах и участвуют в физике. `Scene::bone_positions` рисует линию к ближайшему показанному предку. Без списка (обычные VRM) видны все кости, как раньше.

**Дальше (этап «пальцы/лицо»):** ползунки pose control (данные лежат в `DAZMorphSubBank` бандла `f_mb`, например «Left Thumb Bend» = lThumb1/2/3 на 45/40/68° на единицу) и панель с ними; нужно сверить, что значит число `targetType` (13/14/15 это вращения по осям).


## 3D-редактор поз (лимиты суставов и режимы узлов)

**Лимиты суставов из VaM.** Экспортёр берёт у каждой кости настоящий `ConfigurableJoint` из префаба игры: ось и вторичную ось (рамка сустава), диапазон скрутки по X и пределы качания по Y/Z; кладёт их в расширение `PC_limits` (76 костей у женской, 80 у мужской). Знак скрутки в PhysX обратный (колено «-150..11» на деле сгиб назад на 150°), это поймал тест. Ось переносится через зеркало z как `-M·v` с тем же углом. `model3d` читает расширение, `scene3d::JointLimit` (`joint.rs`) зажимает локальный поворот в форме swing-twist (скрутка по диапазону, качание по эллипсу) относительно позы покоя. Применяется в `set_world_rotation`, то есть и при гизмо, и при IK, и при перетаскивании. Проверено на реальных фигурах (`tests_vam.rs`): колено гнётся только назад, локоть вперёд, рука от плеча не отклоняется больше ~100° от Т-позы, повторное зажатие ничего не меняет.

**Режимы узла (кости).** Вместо галочки «Привязан»: «Следует за родителем» (по умолчанию), «Заморозить» (`Node.hold` = мировая точка) и «Свободно висит» (непривязанная кость, падает по «Пуск»). Заморозка: после перетаскивания кости выше (`enforce_holds` в `hold.rs`) кости между ними поворачиваются так, чтобы удержать замороженную в её точке, насколько позволяют длины костей и лимиты; недостижимо значит она остаётся как можно ближе, ничего не ломается и не растягивается. Если тянуть саму замороженную кость, её точка переносится. Кость, которую пользователь поворачивает руками, свой угол сохраняет. В инспекторе у костей с лимитом есть галочка «Лимит сустава», в панели позы общая «Лимиты суставов» (снять, чтобы позировать без лимитов, как «сброс физики» в VaM).

**Ограничения:** физика «Пуск» пока использует свои приближённые лимиты ±1,6 рад (перенос на шаровые суставы rapier впереди); выталкивание из соседних капсул при позировании тоже ещё нет.
