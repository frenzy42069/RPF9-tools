# RPF9 Models (Blender add-on)

Open RPF9 models directly from Blender. The add-on browses the drawable
models inside a `.rpf` archive and imports the selected model into the scene
through the `rpf9-convert` CLI — i.e., *the plugin throws the model at our
CLI tool and imports the translated GLB*.

## Build and install

1. Build the CLI once: `cd rpf9 && cargo build -p rpf9-convert`
2. Copy `rpf9_addon.py` to Blender's addons directory, e.g.
   `~/.config/blender/5.2/scripts/addons/rpf9_addon.py`
3. In Blender: **Preferences → Add-ons → "RPF9 Models" → enable.**
4. The add-on finds `rpf9-convert` automatically (in `rpf9/target/{debug,release}`),
   or set the path in the add-on preferences, or set the `RPF9_CONVERT`
   environment variable.

## Use

The **RPF9** panel appears in the 3D View sidebar (`N` → RPF9):

1. Pick an archive with the folder icon (or type the `.rpf` path).
2. Press **Refresh models** — `rpf9-convert list-models <rpf>` lists the
   drawable models (name, triangles, vertices).
3. Select a model, press **Import selected model** — it runs
   `rpf9-convert model <rpf> --offset <hex> -o <tmp>.glb` and imports the GLB
   via Blender's built-in glTF importer.

## Known limitations

* The export today writes **mesh geometry + positions only**: no UVs, no
  normals, no materials, no textures in the GLB (gap work — see the
  `rpf9-convert` Phase A/B/C plan: UVs and TTPF model→texture linking).
* A model found this way is identified by its original name
  (e.g. `int_abd_1_gfa_rt_1_shl.#dr` on `levels.rpf`) when available.
