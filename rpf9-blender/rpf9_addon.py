"""rpf9-blender — open RPF9 models directly from Blender.

The add-on adds an "RPF9" panel in the 3D View sidebar. It browses an RPF9
archive's drawable models (via the `rpf9-convert` CLI) and imports the
selected model into the current scene:

    rpf9-convert list-models <rpf> --scan-bytes 0x40000000 --limit 100
    rpf9-convert model <rpf> --offset <hex> -o <tmp>.glb
    bpy.ops.import_scene.gltf(filepath=<tmp>.glb)

Install (Blender 5.x/4.x/3.x):
  1. Build the CLI once: `cd rpf9 && cargo build -p rpf9-convert`
  2. Copy `rpf9_addon.py` to Blender's addons dir, e.g.
     ~/.config/blender/5.2/scripts/addons/rpf9_addon.py
  3. In Blender: Preferences -> Add-ons -> "RPF9 Models" -> enable.
  4. Set the path under the add-on preferences if rpf9-convert is not found
     automatically, or set the `RPF9_CONVERT` environment variable.

Usage:
  * Pick an archive: the folder icon in the RPF9 panel.
  * Press "Refresh models": the drawable models in the archive are listed
    (name, triangle/vertex counts).
  * Select a model, press "Import selected model": it runs rpf9-convert and
    imports the result as a glTF into the scene.

Requires: Blender (the glTF importer ships with Blender) and the
`rpf9-convert` binary.
"""

import json
import os
import re
import shutil
import subprocess
import tempfile

import bpy
from bpy.props import (
    CollectionProperty,
    IntProperty,
    PointerProperty,
    StringProperty,
)
from bpy.types import AddonPreferences, Operator, Panel, PropertyGroup, UIList

DEFAULT_MODEL_LIMIT = 100


def _find_rpf9_convert():
    env = os.environ.get("RPF9_CONVERT")
    if env and os.access(env, os.X_OK):
        return env
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(root, "rpf9", "target", "release", "rpf9-convert"),
        os.path.join(root, "rpf9", "target", "debug", "rpf9-convert"),
        "rpf9-convert",
    ]
    for candidate in candidates:
        if os.path.sep not in candidate:
            candidate = shutil.which(candidate)
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return None


def _rpf9_convert_path():
    try:
        entry = bpy.context.preferences.addons.get(__name__)
        if entry is not None and entry.preferences is not None:
            path = entry.preferences.rpf9_convert_path
            if path and os.access(path, os.X_OK):
                return path
    except Exception:
        pass
    return ""


def _run_cli(args, timeout=900):
    exe = _find_rpf9_convert() or _rpf9_convert_path()
    if not exe or not os.access(exe, os.X_OK):
        raise RuntimeError(
            "rpf9-convert not found: build it (`cd rpf9 && cargo build -p rpf9-convert`), "
            "set RPF9_CONVERT, or configure the add-on path in preferences."
        )
    try:
        result = subprocess.run(
            [exe, *args],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=True,
        )
    except subprocess.TimeoutExpired as e:
        raise RuntimeError("rpf9-convert timed out") from e
    except subprocess.CalledProcessError as e:
        detail = (e.stderr or e.stdout or f"rpf9-convert exited with {e.returncode}").strip()
        raise RuntimeError(detail) from e
    except OSError as e:
        raise RuntimeError(f"cannot run rpf9-convert at {exe}: {e}") from e
    return result.stdout


class Rpf9ModelProp(PropertyGroup):
    name: StringProperty(name="Model name")
    offset: StringProperty(name="Offset (hex)")
    vertices: IntProperty(name="Vertices")
    triangles: IntProperty(name="Triangles")


class Rpf9ArchiveProp(PropertyGroup):
    path: StringProperty(name="Archive path (.rpf)")
    index: IntProperty(name="Selected index", default=-1)
    models: CollectionProperty(type=Rpf9ModelProp)


class Rpf9AddonPreferences(AddonPreferences):
    bl_idname = __name__
    rpf9_convert_path: StringProperty(
        name="rpf9-convert path",
        description="Path to the rpf9-convert executable (cargo build -p rpf9-convert in rpf9/)",
        default="",
    )


class Rpf9PickArchive(Operator):
    """Open a file browser to select an RPF9 archive (.rpf)."""

    bl_idname = "rpf9.pick_archive"
    bl_label = "Pick archive"
    bl_options = {"REGISTER"}

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        archive = context.scene.rpf9_models
        archive.path = self.filepath
        return {"CANCELLED"}


class Rpf9RefreshModels(Operator):
    """Run rpf9-convert list-models and populate the model list."""

    bl_idname = "rpf9.refresh_models"
    bl_label = "Refresh models"

    @classmethod
    def poll(cls, context):
        return bool(context.scene.rpf9_models.path)

    def execute(self, context):
        archive = context.scene.rpf9_models
        try:
            # Scan the whole file: drawable models sit at various offsets and
            # the catalog stops early once it has found enough models.
            scan_bytes = str(os.path.getsize(archive.path))
            out = _run_cli(
                [
                    "list-models",
                    archive.path,
                    "--scan-bytes",
                    scan_bytes,
                    "--limit",
                    str(DEFAULT_MODEL_LIMIT),
                ]
            )
            models = json.loads(out)
        except Exception as error:
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}
        archive.models.clear()
        archive.index = -1
        for item in models:
            model = archive.models.add()
            model.name = item["name"]
            model.offset = hex(int(item["offset"]))
            model.vertices = int(item["vertices"])
            model.triangles = int(item["triangles"])
        if archive.models:
            archive.index = 0
        return {"FINISHED"}


class Rpf9ImportSelected(Operator):
    """Run rpf9-convert model for the selected model and import the GLB."""

    bl_idname = "rpf9.import_selected"
    bl_label = "Import selected model"

    @classmethod
    def poll(cls, context):
        archive = context.scene.rpf9_models
        return 0 <= archive.index < len(archive.models)

    def execute(self, context):
        archive = context.scene.rpf9_models
        model = archive.models[archive.index]
        safe_name = re.sub(r"[^0-9A-Za-z_.-]", "_", model.name) or "model"
        with tempfile.TemporaryDirectory(prefix="rpf9_blender_") as tmp:
            glb_path = os.path.join(tmp, f"{safe_name}.glb")
            try:
                _run_cli(["model", archive.path, "--offset", model.offset, "-o", glb_path])
            except Exception as error:
                self.report({"ERROR"}, f"rpf9-convert model failed: {error}")
                return {"CANCELLED"}
            try:
                bpy.ops.import_scene.gltf(filepath=glb_path)
            except Exception as error:
                self.report({"ERROR"}, f"glTF import failed: {error}")
                return {"CANCELLED"}
            self.report({"INFO"}, f"Imported {model.name}")
            return {"FINISHED"}


class RPF9_UL_models(UIList):
    """List of models found in the selected RPF9 archive."""

    def draw_item(self, context, layout, data, item, icon, active_data, active_item, index):
        row = layout.row(align=True)
        row.label(text=item.name or "(unnamed)", icon="OBJECT_DATA")
        row.label(text=f"{item.triangles} tris · {item.vertices} verts")


class Rpf9ModelsPanel(Panel):
    bl_label = "RPF9 Models"
    bl_idname = "RPF9_PT_models"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "RPF9"

    def draw(self, context):
        archive = context.scene.rpf9_models
        layout = self.layout
        layout.label(text="Archive:")
        row = layout.row(align=True)
        row.prop(archive, "path", text="")
        row.operator("rpf9.pick_archive", icon="FILEBROWSER", text="")
        row.operator("rpf9.refresh_models", icon="FILE_REFRESH", text="")
        if not archive.path:
            layout.label(text="Pick an archive above to list models.", icon="INFO")
            return
        layout.label(text=f"{len(archive.models)} models")
        layout.template_list(
            "RPF9_UL_models", "", archive, "models", archive, "index", rows=6
        )
        row = layout.row()
        row.operator("rpf9.import_selected", icon="IMPORT")


CLASSES = (
    Rpf9ModelProp,
    Rpf9ArchiveProp,
    Rpf9AddonPreferences,
    Rpf9PickArchive,
    Rpf9RefreshModels,
    Rpf9ImportSelected,
    RPF9_UL_models,
    Rpf9ModelsPanel,
)


def register():
    for cls in CLASSES:
        try:
            bpy.utils.register_class(cls)
        except RuntimeError:
            pass  # already registered
    bpy.types.Scene.rpf9_models = PointerProperty(type=Rpf9ArchiveProp)


def unregister():
    try:
        del bpy.types.Scene.rpf9_models
    except AttributeError:
        pass
    for cls in reversed(CLASSES):
        try:
            bpy.utils.unregister_class(cls)
        except RuntimeError:
            pass
