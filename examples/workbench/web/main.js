// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import {
  DEFAULT_RECIPE,
  SPECIES,
  validateRecipe,
  parseHash,
  recipeHash,
} from "./recipe.js";

const $ = (id) => document.getElementById(id);
const format = (value) => new Intl.NumberFormat("en").format(value);
const scene = new THREE.Scene();
scene.background = new THREE.Color("#eff1e8");
const camera = new THREE.PerspectiveCamera(35, 1, 0.01, 1000);
const renderer = new THREE.WebGLRenderer({
  antialias: true,
  preserveDrawingBuffer: true,
});
renderer.setPixelRatio(Math.min(devicePixelRatio, 1.75));
renderer.shadowMap.enabled = true;
// This specimen is static; refresh shadows only when it or the light changes.
renderer.shadowMap.autoUpdate = false;
renderer.shadowMap.type = THREE.PCFShadowMap;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.35;
$("viewport").append(renderer.domElement);
const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.maxPolarAngle = Math.PI * 0.52;
controls.autoRotateSpeed = 0.6;
const sky = new THREE.HemisphereLight(0xe9f1df, 0x72765e, 2.5);
scene.add(sky);
const sun = new THREE.DirectionalLight(0xfff0d6, 3.5);
sun.castShadow = true;
sun.shadow.mapSize.set(2048, 2048);
sun.shadow.normalBias = 0.03;
sun.shadow.bias = -0.0001;
scene.add(sun, sun.target);
const fill = new THREE.DirectionalLight(0xe8f2ff, 1.2);
fill.position.set(-10, 12, -15);
scene.add(fill);
const floor = new THREE.Mesh(
  new THREE.PlaneGeometry(2000, 2000),
  new THREE.MeshStandardMaterial({ color: "#eff1e8", roughness: 1 }),
);
floor.rotation.x = -Math.PI / 2;
floor.receiveShadow = true;
floor.position.y = -0.015;
scene.add(floor);
const grid = new THREE.GridHelper(40, 40, 0xb9c3a9, 0xd6dccb);
grid.material.transparent = true;
grid.material.opacity = 0.18;
grid.position.y = -0.008;
scene.add(grid);
const loader = new GLTFLoader();
let current = null,
  worker = null,
  request = 0,
  mode = "canopy",
  dirty = false,
  selected = null;
let selectedSpecies = "birch";
let lastFrame = performance.now(),
  frameMs = 0;
const clay = new THREE.MeshStandardMaterial({
  color: 0xa69d83,
  roughness: 0.85,
  side: THREE.DoubleSide,
});
const wire = new THREE.MeshBasicMaterial({ color: 0x315a40, wireframe: true });
const highlightMaterial = new THREE.MeshBasicMaterial({
  color: 0xe7ac40,
  side: THREE.DoubleSide,
  polygonOffset: true,
  polygonOffsetFactor: -2,
  polygonOffsetUnits: -2,
});

function status(text, error = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", error);
}
function setBusy(busy) {
  $("loading").hidden = !busy;
  $("cancel").hidden = !busy;
  $("grow").disabled = busy;
  $("grow").innerHTML = busy ? "Generating…" : "Generate tree <span>↗</span>";
}
function readRecipe() {
  return validateRecipe({
    version: 1,
    species: selectedSpecies,
    seed: Number($("seed").value),
    density: Number($("density").value) / 100,
    leaf_size: Number($("leaf-size").value) / 100,
  });
}
function setForm(recipe) {
  selectedSpecies = recipe.species;
  document.querySelectorAll("[data-species]").forEach((button) => {
    const active = button.dataset.species === recipe.species;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", active);
  });
  $("seed").value = recipe.seed;
  $("density").value = recipe.density * 100;
  $("leaf-size").value = recipe.leaf_size * 100;
  updateOutputs();
}
function updateOutputs() {
  $("density-value").textContent = $("density").value + "%";
  $("leaf-size-value").textContent = $("leaf-size").value + "%";
}
function markDirty() {
  updateOutputs();
  dirty = true;
  status(
    "Settings changed. Generate to apply; exports use the displayed specimen.",
  );
}
function dispose(root) {
  const geometries = new Set(),
    materials = new Set(),
    textures = new Set();
  root.traverse((object) => {
    if (!object.isMesh) return;
    geometries.add(object.geometry);
    const material = object.userData.originalMaterial ?? object.material;
    for (const item of Array.isArray(material) ? material : [material]) {
      materials.add(item);
      for (const value of Object.values(item))
        if (value?.isTexture) textures.add(value);
    }
    if (object.isInstancedMesh) object.dispose();
  });
  geometries.forEach((value) => value.dispose());
  materials.forEach((value) => value.dispose());
  textures.forEach((value) => {
    value.source?.data?.close?.();
    value.dispose();
  });
}
function applyMode() {
  if (!current) return;
  current.root.traverse((object) => {
    if (!object.isMesh) return;
    const foliage = object.isInstancedMesh;
    object.visible = mode !== "structure" || !foliage;
    object.material =
      mode === "wire"
        ? wire
        : mode === "structure"
          ? clay
          : object.userData.originalMaterial;
  });
  $("hint").textContent =
    mode === "structure"
      ? "Click a branch to inspect its source identity · scroll to approach"
      : "Drag to orbit · scroll to explore · right-drag to pan";
  renderer.shadowMap.needsUpdate = true;
}
function fitBox(box) {
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const extent = Math.max(
    size.y,
    size.x / camera.aspect,
    size.z / camera.aspect,
    0.1,
  );
  const distance =
    (extent / (2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)))) * 1.3;
  const direction = new THREE.Vector3(0.8, 0.24, 1).normalize();
  camera.position.copy(center).addScaledVector(direction, distance);
  camera.near = Math.max(0.002, distance / 2000);
  camera.far = Math.max(1000, distance * 20);
  camera.updateProjectionMatrix();
  controls.target.copy(center);
  controls.minDistance = 0.02;
  controls.maxDistance = Math.max(200, distance * 4);
  controls.update();
}
function positionSun() {
  const size = current?.bounds.getSize(new THREE.Vector3()).length() ?? 30;
  const angle = THREE.MathUtils.degToRad(Number($("sun").value));
  sun.position.set(Math.cos(angle) * size, size * 1.3, Math.sin(angle) * size);
  sun.target.position.set(0, size * 0.2, 0);
  const span = size * 0.6;
  Object.assign(sun.shadow.camera, {
    left: -span,
    right: span,
    top: span,
    bottom: -span,
    near: 0.1,
    far: size * 5,
  });
  sun.shadow.camera.updateProjectionMatrix();
  sun.shadow.normalBias = size * 0.0006;
  renderer.shadowMap.needsUpdate = true;
}
async function grow(recipe) {
  const id = ++request;
  worker?.terminate();
  worker = new Worker(new URL("./worker.js", import.meta.url), {
    type: "module",
  });
  setBusy(true);
  status("Starting the Rust generator…");
  const job = worker;
  job.onerror = (event) => {
    if (id === request)
      fail(
        event.message ||
          "The generation worker stopped. Try a lower foliage density.",
      );
  };
  job.onmessage = async ({ data }) => {
    if (id !== request) return;
    if (data.type === "progress") {
      status(data.text);
      return;
    }
    if (data.type === "error") {
      fail(data.message);
      return;
    }
    if (data.type !== "result") return;
    status("Loading geometry and materials into the preview…");
    const start = performance.now();
    try {
      const gltf = await loader.parseAsync(data.glb, "");
      if (id !== request) {
        dispose(gltf.scene);
        return;
      }
      gltf.scene.traverse((object) => {
        if (!object.isMesh) return;
        object.castShadow = true;
        object.receiveShadow = true;
        object.userData.originalMaterial = object.material;
        if (object.isInstancedMesh) {
          object.computeBoundingBox();
          object.computeBoundingSphere();
        }
      });
      const bounds = new THREE.Box3().setFromObject(gltf.scene);
      clearSelection();
      const previous = current;
      current = {
        root: gltf.scene,
        bytes: data.glb,
        report: data.report,
        bounds,
      };
      current.report.timings.preview_load_ms = performance.now() - start;
      scene.add(current.root);
      if (previous) {
        scene.remove(previous.root);
        dispose(previous.root);
      }
      const sameSpecies = previous?.report.recipe.species === recipe.species;
      if (!sameSpecies) fitBox(bounds);
      const size = bounds.getSize(new THREE.Vector3());
      current.report.bounds = {
        min: bounds.min.toArray(),
        max: bounds.max.toArray(),
      };
      $("specimen-name").textContent = SPECIES[recipe.species][0];
      $("specimen-latin").textContent = SPECIES[recipe.species][1];
      $("specimen-seed").textContent =
        `Seed ${String(recipe.seed).padStart(6, "0")} · detailed geometry`;
      $("height").textContent = size.y.toFixed(1) + " m";
      for (const key of ["branches", "placements", "templates"])
        $(key).textContent = format(data.report.counts[key]);
      $("export-info").textContent =
        `${(data.glb.byteLength / 1048576).toFixed(1)} MB · instanced GLB · metres, Y up`;
      for (const key of [
        "export",
        "save-recipe",
        "screenshot",
        "share",
        "diagnostics",
      ])
        $(key).disabled = false;
      try {
        dirty = JSON.stringify(readRecipe()) !== JSON.stringify(recipe);
      } catch {
        dirty = true;
      }
      positionSun();
      applyMode();
      setBusy(false);
      status(
        `Generated in ${(data.report.timings.generation_export_ms / 1000).toFixed(1)} s; loaded in ${(data.report.timings.preview_load_ms / 1000).toFixed(1)} s. ${dirty ? "Unapplied parameter changes." : ""}`,
      );
      history.replaceState(null, "", recipeHash(recipe));
      job.terminate();
      if (worker === job) worker = null;
    } catch (error) {
      if (id === request) fail(error.message);
    }
  };
  job.postMessage({ recipe });
}
function fail(message) {
  worker?.terminate();
  worker = null;
  setBusy(false);
  status(
    `${message} ${current ? "Your previous specimen is still available." : "Adjust the settings and try again."}`,
    true,
  );
}
function download(blob, name) {
  const url = URL.createObjectURL(blob),
    link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function name(extension) {
  const r = current.report.recipe;
  return `sylva-${r.species}-${r.seed}.${extension}`;
}
function downloadJSON(data, filename) {
  download(
    new Blob([JSON.stringify(data, null, 2) + "\n"], {
      type: "application/json",
    }),
    filename,
  );
}
function clearSelection() {
  if (selected) {
    selected.mesh.removeFromParent();
    selected.mesh.geometry.dispose();
  }
  selected = null;
  $("selection").hidden = true;
}
function inspectBranch(event) {
  if (!current || mode !== "structure") return;
  const rect = renderer.domElement.getBoundingClientRect();
  const pointer = new THREE.Vector2(
    ((event.clientX - rect.left) / rect.width) * 2 - 1,
    (-(event.clientY - rect.top) / rect.height) * 2 + 1,
  );
  const ray = new THREE.Raycaster();
  ray.setFromCamera(pointer, camera);
  const meshes = [];
  current.root.traverse((o) => {
    if (o.isMesh && !o.isInstancedMesh && o.visible) meshes.push(o);
  });
  const hit = ray.intersectObjects(meshes, false)[0];
  if (!hit) return;
  const source = hit.object.geometry,
    attribute = source.getAttribute("_branch");
  if (!attribute) {
    status("This mesh has no branch provenance attribute.", true);
    return;
  }
  const branchIndex = Math.round(attribute.getX(hit.face.a));
  const branch = current.report.branches[branchIndex];
  if (!branch) return;
  clearSelection();
  const indices = [],
    original = source.index;
  for (let i = 0; i < original.count; i += 3) {
    const a = original.getX(i);
    if (Math.round(attribute.getX(a)) === branchIndex)
      indices.push(a, original.getX(i + 1), original.getX(i + 2));
  }
  const geometry = new THREE.BufferGeometry();
  // Copy only this branch so selection disposal cannot invalidate source buffers.
  const positions = source.getAttribute("position"),
    points = [];
  for (const i of indices)
    points.push(positions.getX(i), positions.getY(i), positions.getZ(i));
  geometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(points, 3),
  );
  const mesh = new THREE.Mesh(geometry, highlightMaterial);
  mesh.matrixAutoUpdate = false;
  mesh.matrix.copy(hit.object.matrixWorld);
  scene.add(mesh);
  selected = { mesh, branch };
  $("selection").hidden = false;
  $("branch-title").textContent =
    branch.order === 0 ? "Main stem" : `Branch · order ${branch.order}`;
  $("branch-id").textContent = branch.id;
  $("branch-parent").textContent = branch.parent
    ? `Parent ${branch.parent}`
    : "Root of the botanical skeleton";
}
let down = null;
renderer.domElement.addEventListener("pointerdown", (event) => {
  down = [event.clientX, event.clientY];
});
renderer.domElement.addEventListener("pointerup", (event) => {
  if (
    down &&
    Math.hypot(event.clientX - down[0], event.clientY - down[1]) < 4 &&
    event.button === 0
  )
    inspectBranch(event);
  down = null;
});
$("clear-selection").onclick = clearSelection;
$("focus-branch").onclick = () => {
  if (selected) fitBox(new THREE.Box3().setFromObject(selected.mesh));
};
$("fit").onclick = () => {
  if (current) fitBox(current.bounds);
};
$("rotate").onclick = () => {
  controls.autoRotate = !controls.autoRotate;
  $("rotate").setAttribute("aria-pressed", controls.autoRotate);
};
$("sun").oninput = positionSun;
document.querySelectorAll("[data-mode]").forEach(
  (button) =>
    (button.onclick = () => {
      mode = button.dataset.mode;
      clearSelection();
      document.querySelectorAll("[data-mode]").forEach((b) => {
        const active = b === button;
        b.classList.toggle("active", active);
        b.setAttribute("aria-pressed", active);
      });
      applyMode();
    }),
);
document.querySelectorAll("[data-species]").forEach(
  (button) =>
    (button.onclick = () => {
      selectedSpecies = button.dataset.species;
      document.querySelectorAll("[data-species]").forEach((b) => {
        b.classList.toggle("active", b === button);
        b.setAttribute("aria-pressed", b === button);
      });
      markDirty();
    }),
);
$("recipe-form").onsubmit = (event) => {
  event.preventDefault();
  try {
    grow(readRecipe());
  } catch (error) {
    status(error.message, true);
  }
};
for (const id of ["seed", "density", "leaf-size"]) $(id).oninput = markDirty;
$("shuffle").onclick = () => {
  $("seed").value = crypto.getRandomValues(new Uint32Array(1))[0];
  markDirty();
};
$("cancel").onclick = () => {
  ++request;
  worker?.terminate();
  worker = null;
  setBusy(false);
  status("Generation cancelled. The displayed specimen is unchanged.");
};
$("save-recipe").onclick = () =>
  downloadJSON(current.report.recipe, name("recipe.json"));
$("export").onclick = () =>
  download(
    new Blob([current.bytes], { type: "model/gltf-binary" }),
    name("glb"),
  );
$("screenshot").onclick = () => {
  renderer.render(scene, camera);
  renderer.domElement.toBlob((blob) => {
    if (blob) download(blob, name("png"));
  });
};
$("share").onclick = async () => {
  const url = new URL(location.href);
  url.hash = recipeHash(current.report.recipe);
  try {
    await navigator.clipboard.writeText(url.href);
    status(
      "Specimen link copied. It will regenerate this recipe on another device.",
    );
  } catch {
    status("Copy the address bar to share this specimen.");
  }
};
$("load-recipe").onclick = () => $("recipe-file").click();
$("recipe-file").onchange = async (event) => {
  const file = event.target.files[0];
  event.target.value = "";
  if (!file) return;
  try {
    if (file.size > 16384)
      throw new Error("Recipe files must be smaller than 16 KB.");
    const recipe = validateRecipe(JSON.parse(await file.text()));
    setForm(recipe);
    grow(recipe);
  } catch (error) {
    status(error.message + " The displayed specimen is unchanged.", true);
  }
};
$("diagnostics").onclick = () => {
  const c = current.report.counts,
    t = current.report.timings;
  const rows = [
    ["Source branches", format(c.branches)],
    ["Leaf placements", format(c.placements)],
    ["Stored templates", c.templates],
    ["Stored triangles", format(c.stored_triangles)],
    ["Triangles with all instances", format(c.expanded_triangles)],
    [
      "Generation + materials + export",
      (t.generation_export_ms / 1000).toFixed(2) + " s",
    ],
    ["Preview load", (t.preview_load_ms / 1000).toFixed(2) + " s"],
    ["Frame interval (CPU wall clock)", frameMs.toFixed(1) + " ms"],
    ["Renderer draw calls", renderer.info.render.calls],
    ["Renderer triangles", format(renderer.info.render.triangles)],
    ["GLB size", (c.glb_bytes / 1048576).toFixed(2) + " MB"],
  ];
  $("report-values").replaceChildren(
    ...rows.flatMap(([key, value]) => {
      const dt = document.createElement("dt"),
        dd = document.createElement("dd");
      dt.textContent = key;
      dd.textContent = value;
      return [dt, dd];
    }),
  );
  $("report-dialog").showModal();
};
$("close-report").onclick = () => $("report-dialog").close();
$("download-report").onclick = () =>
  downloadJSON(
    {
      ...current.report,
      preview: {
        renderer: "Three.js",
        revision: THREE.REVISION,
        mode,
        draw_calls: renderer.info.render.calls,
        triangles: renderer.info.render.triangles,
        frame_interval_ms: frameMs,
        camera: {
          position: camera.position.toArray(),
          target: controls.target.toArray(),
        },
      },
    },
    name("report.json"),
  );
new ResizeObserver(() => {
  const { width, height } = $("viewport").getBoundingClientRect();
  renderer.setSize(width, height);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}).observe($("viewport"));
renderer.setAnimationLoop(() => {
  const now = performance.now();
  frameMs = frameMs * 0.95 + (now - lastFrame) * 0.05;
  lastFrame = now;
  controls.update();
  renderer.render(scene, camera);
});
camera.position.set(25, 14, 30);
controls.target.set(0, 9, 0);
controls.update();
try {
  const recipe = parseHash(location.hash);
  setForm(recipe);
  grow(recipe);
} catch (error) {
  setForm(DEFAULT_RECIPE);
  setBusy(false);
  status(error.message + " Select parameters and generate to continue.", true);
}
