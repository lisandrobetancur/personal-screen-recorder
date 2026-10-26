import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { setLanguage, t, translatePage } from "./i18n";
import { mockInvoke, mockPickFolder } from "./mock";
import { type Check, type HealthReport, Watchdog } from "./watchdog";

const preview = !isTauri() && import.meta.env.DEV;
const invoke = (preview ? mockInvoke : tauriInvoke) as typeof tauriInvoke;

type SourceKind = "display" | "window";
type Theme = "system" | "light" | "dark";

interface Source {
  kind: SourceKind;
  id: number;
  name: string;
  width: number;
  height: number;
}

interface Microphone {
  id: string;
  name: string;
  isDefault: boolean;
}

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;

const els = {
  themeButtons: document.querySelectorAll<HTMLButtonElement>(".theme button"),
  folds: document.querySelectorAll<HTMLElement>(".fold"),
  refresh: $<HTMLButtonElement>("#refresh"),
  tabs: document.querySelectorAll<HTMLButtonElement>(".tab"),
  sources: $<HTMLUListElement>("#sources"),
  systemAudio: $<HTMLInputElement>("#system-audio"),
  microphone: $<HTMLInputElement>("#microphone"),
  microphoneWrap: $<HTMLDivElement>("#microphone-wrap"),
  microphoneId: $<HTMLSelectElement>("#microphone-id"),
  fps: $<HTMLDivElement>("#fps"),
  showCursor: $<HTMLInputElement>("#show-cursor"),
  showClicks: $<HTMLInputElement>("#show-clicks"),
  excludeSelf: $<HTMLInputElement>("#exclude-self"),
  outputDir: $<HTMLSpanElement>("#output-dir"),
  changeDir: $<HTMLButtonElement>("#change-dir"),
  record: $<HTMLButtonElement>("#record"),
  recordLabel: $<HTMLSpanElement>("#record-label"),
  timer: $<HTMLSpanElement>("#timer"),
  status: $<HTMLParagraphElement>("#status"),
  last: $<HTMLDivElement>("#last"),
  lastName: $<HTMLSpanElement>("#last-name"),
  lastMeta: $<HTMLElement>("#last-meta"),
  reveal: $<HTMLButtonElement>("#reveal"),
  alert: $<HTMLDivElement>("#alert"),
  alertItems: $<HTMLElement>("#alert-items"),
  warnSilence: $<HTMLInputElement>("#warn-silence"),
  alertReal: $<HTMLButtonElement>("#alert-real"),
  alertStop: $<HTMLButtonElement>("#alert-stop"),
};

const state = {
  kind: "display" as SourceKind,
  sources: [] as Source[],
  selected: null as Source | null,
  fps: 60,
  recording: false,
  busy: false,
  startedAt: 0,
  timerId: 0,
  outputDir: "",
  lastPath: "",
  watchdog: null as Watchdog | null,
  watchId: 0,
  lastHealth: null as HealthReport | null,
};

// ---------- Tema ----------

function readTheme(): Theme {
  try {
    const t = localStorage.getItem("theme");
    if (t === "light" || t === "dark") return t;
  } catch {}
  return "system";
}

function applyTheme(theme: Theme) {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
  els.themeButtons.forEach((b) => b.setAttribute("aria-checked", String(b.dataset.theme === theme)));
  try {
    if (theme === "system") localStorage.removeItem("theme");
    else localStorage.setItem("theme", theme);
  } catch {}
  // La barra de título nativa sigue al mismo tema.
  if (isTauri()) {
    getCurrentWindow()
      .setTheme(theme === "system" ? null : theme)
      .catch(() => {});
  }
}

// ---------- Secciones plegables ----------

/** Abre una sección y cierra las demás: solo una abierta a la vez. */
function openSection(key: string | null) {
  els.folds.forEach((fold) => {
    const open = fold.dataset.section === key;
    fold.classList.toggle("open", open);
    fold.querySelector(".fold-toggle")!.setAttribute("aria-expanded", String(open));
    fold.querySelector(".fold-body")!.toggleAttribute("inert", !open);
  });
}

function setSummary(key: string, text: string) {
  $<HTMLSpanElement>(`#summary-${key}`).textContent = text;
}

/** Resumen de cada sección, visible aunque esté cerrada. */
function updateSummaries() {
  setSummary("source", state.selected ? splitName(state.selected)[0] : t("source.choose"));
  const audio = [els.systemAudio.checked && t("audio.summarySystem"), els.microphone.checked && t("audio.summaryMic")].filter(Boolean);
  setSummary("audio", audio.length ? audio.join(" + ") : t("audio.none"));
  setSummary("video", `${state.fps} fps`);
}

// ---------- Alto de la ventana ----------

/** Alto que necesita el contenido, sin espacio vacío al plegar secciones. */
function contentHeight() {
  const main = document.querySelector("main")!;
  const visible = [...main.children].filter((el) => (el as HTMLElement).offsetParent !== null) as HTMLElement[];
  const last = visible[visible.length - 1];
  const padding = parseFloat(getComputedStyle(main).paddingBottom);
  const mainHeight = last ? last.offsetTop + last.offsetHeight + padding - main.offsetTop : 0;
  return document.querySelector<HTMLElement>(".topbar")!.offsetHeight + mainHeight + $(".dock").offsetHeight;
}

let fitPending = false;
/** Alto de la ventana que no es contenido (en Mac, la barra de título). Se mide solo. */
let chromeHeight = 0;

/** Ajusta el alto de la ventana al contenido (el ancho lo decide la persona). */
function fitWindow() {
  if (!isTauri() || fitPending) return;
  fitPending = true;
  requestAnimationFrame(async () => {
    try {
      // Dos pasadas como máximo: la segunda corrige la barra de título.
      for (let pass = 0; pass < 2; pass++) {
        const needed = Math.min(Math.ceil(contentHeight()), screen.availHeight - 40);
        const missing = needed - window.innerHeight;
        if (Math.abs(missing) <= 1) break;
        const requested = needed + chromeHeight;
        await getCurrentWindow().setSize(new LogicalSize(window.innerWidth, requested));
        await new Promise((r) => setTimeout(r, 50));
        // Lo que faltó tras pedir `requested` es lo que ocupa la ventana fuera del contenido.
        const shortfall = needed - window.innerHeight;
        if (shortfall > 0 && shortfall < 100) chromeHeight += shortfall;
      }
    } catch {
      // Si no se puede cambiar el tamaño, la ventana queda como está y main se desplaza.
    } finally {
      fitPending = false;
    }
  });
}

// ---------- Fuentes ----------

function setStatus(text: string, error = false) {
  els.status.textContent = text;
  els.status.classList.toggle("error", error);
}

function sameSource(a: Source, b: Source) {
  return a.kind === b.kind && a.id === b.id;
}

/** "Google Chrome — Mi pestaña" → ["Google Chrome", "Mi pestaña"]. */
function splitName(s: Source): [string, string] {
  const i = s.name.indexOf(" — ");
  if (s.kind === "display") return [i >= 0 ? s.name.slice(0, i) : s.name, ""];
  return i >= 0 ? [s.name.slice(0, i), s.name.slice(i + 3)] : [s.name, ""];
}

function renderSources() {
  const list = state.sources.filter((s) => s.kind === state.kind);
  if (state.selected && !list.some((s) => sameSource(s, state.selected!))) {
    state.selected = list.find((s) => s.kind === state.selected!.kind && s.id === state.selected!.id) ?? null;
  }
  if (!state.selected && state.kind === "display" && list.length > 0) {
    state.selected = list[0];
  }

  els.sources.replaceChildren(
    ...list.map((s) => {
      const [title, detail] = splitName(s);
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.className = "source";
      button.setAttribute("aria-pressed", String(!!state.selected && sameSource(s, state.selected)));
      button.innerHTML = `
        <svg class="source-icon"><use href="#i-${s.kind === "display" ? "monitor" : "window"}" /></svg>
        <span class="source-text"><span class="title"></span><small class="detail"></small></span>
        <span class="size">${s.width}×${s.height}</span>`;
      button.querySelector(".title")!.textContent = title;
      button.querySelector(".detail")!.textContent = detail;
      button.addEventListener("click", () => {
        state.selected = s;
        renderSources();
      });
      li.append(button);
      return li;
    }),
  );
  if (list.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = state.kind === "display" ? t("source.noDisplays") : t("source.noWindows");
    els.sources.append(li);
  }
  updateControls();
}

function updateControls() {
  els.record.disabled = state.busy || (!state.recording && !state.selected);
  els.record.classList.toggle("recording", state.recording);
  document.body.classList.toggle("is-recording", state.recording);
  els.recordLabel.textContent = t(state.busy && state.recording ? "record.saving" : state.recording ? "record.stop" : "record.start");
  const lock = state.recording || state.busy;
  for (const el of [
    els.refresh,
    els.systemAudio,
    els.microphone,
    els.showCursor,
    els.showClicks,
    els.excludeSelf,
    els.warnSilence,
    els.microphoneId,
    els.changeDir,
  ]) {
    el.disabled = lock;
  }
  els.microphoneWrap.hidden = !els.microphone.checked;
  els.sources.classList.toggle("locked", lock);
  els.tabs.forEach((t) => (t.disabled = lock));
  els.fps.querySelectorAll("button").forEach((b) => (b.disabled = lock));
  updateSummaries();
}

async function loadSources() {
  els.refresh.classList.add("spinning");
  try {
    state.sources = await invoke<Source[]>("list_sources");
    setStatus("");
  } catch (e) {
    state.sources = [];
    setStatus(String(e), true);
  } finally {
    els.refresh.classList.remove("spinning");
  }
  renderSources();
}

async function loadMicrophones() {
  const mics = await invoke<Microphone[]>("list_microphones");
  els.microphoneId.replaceChildren(
    ...mics.map((m) => {
      const option = document.createElement("option");
      option.value = m.id;
      option.textContent = m.isDefault ? `${m.name} (${t("audio.default")})` : m.name;
      option.selected = m.isDefault;
      return option;
    }),
  );
  if (mics.length === 0) {
    els.microphone.checked = false;
    els.microphone.closest(".option")?.classList.add("unavailable");
  }
  updateControls();
}

// ---------- Carpeta de grabaciones ----------

function showOutputDir(dir: string) {
  state.outputDir = dir;
  els.changeDir.title = t("folder.tooltip", { dir });
  els.outputDir.textContent = dir.replace(/^\/Users\/[^/]+/, "~");
}

async function changeOutputDir() {
  const picked = preview
    ? await mockPickFolder()
    : await pickFolder({ directory: true, defaultPath: state.outputDir, title: t("folder.pickerTitle") });
  if (typeof picked !== "string") return;
  try {
    showOutputDir(await invoke<string>("set_output_dir", { path: picked }));
    setStatus(t("folder.changed"));
  } catch (e) {
    setStatus(String(e), true);
  }
}

// ---------- Avisos de grabación vacía ----------

const alertTexts = {
  black: ["alert.blackTitle", "alert.blackText"],
  microphone: ["alert.micTitle", "alert.micText"],
  system: ["alert.systemTitle", "alert.systemText"],
  microphoneLost: ["alert.micLostTitle", "alert.micLostText"],
  systemLost: ["alert.systemLostTitle", "alert.systemLostText"],
} as const satisfies Record<Check, readonly [string, string]>;

function startWatching() {
  // Sin «Avisar si falta el sonido» solo se vigila la imagen.
  const warn = els.warnSilence.checked;
  state.watchdog = new Watchdog({ system: warn && els.systemAudio.checked, microphone: warn && els.microphone.checked });
  state.watchId = window.setInterval(checkHealth, 1000);
}

function stopWatching() {
  window.clearInterval(state.watchId);
  state.watchdog = null;
  showAlert([]);
}

async function checkHealth() {
  const watchdog = state.watchdog;
  if (!watchdog) return;
  const report = await invoke<HealthReport | null>("recording_health").catch(() => null);
  if (!report || state.watchdog !== watchdog) return;
  state.lastHealth = report;
  const before = watchdog.questions;
  const questions = watchdog.update(report);
  if (questions.join() !== before.join()) {
    showAlert(questions);
    const added = questions.filter((q) => !before.includes(q));
    if (added.length) notify(added);
  }
}

/** Muestra todas las preguntas abiertas en un solo aviso. */
function showAlert(questions: Check[]) {
  els.alert.hidden = questions.length === 0;
  els.alertItems.replaceChildren(
    ...questions.map((q) => {
      const [title, text] = alertTexts[q];
      const item = document.createElement("div");
      item.className = "alert-item";
      const strong = document.createElement("strong");
      strong.textContent = t(title);
      const small = document.createElement("small");
      small.textContent = t(text);
      item.append(strong, small);
      return item;
    }),
  );
}

/** Notificación del sistema: mientras se graba, la ventana suele estar detrás. */
async function notify(questions: Check[]) {
  if (preview) return;
  const [title, text] = alertTexts[questions[0]];
  const body = questions.length === 1 ? t(text) : questions.slice(1).map((q) => t(alertTexts[q][0])).join(" · ");
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title: t(title), body });
  } catch {
    // Sin notificaciones queda el aviso dentro de la app.
  }
  getCurrentWindow().requestUserAttention(2).catch(() => {});
}

// ---------- Grabación ----------

function formatElapsed(ms: number) {
  const total = Math.floor(ms / 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(total / 3600))}:${pad(Math.floor(total / 60) % 60)}:${pad(total % 60)}`;
}

async function toggleRecording() {
  const wasRecording = state.recording;
  state.busy = true;
  updateControls();
  try {
    if (!wasRecording) {
      const s = state.selected!;
      await invoke<string>("start_recording", {
        options: {
          sourceKind: s.kind,
          sourceId: s.id,
          fps: state.fps,
          systemAudio: els.systemAudio.checked,
          microphone: els.microphone.checked,
          microphoneId: els.microphone.checked ? els.microphoneId.value || null : null,
          showCursor: els.showCursor.checked,
          showClicks: els.showClicks.checked,
          excludeSelf: els.excludeSelf.checked,
          outputDir: state.outputDir || null,
        },
      });
      state.recording = true;
      state.startedAt = Date.now();
      els.timer.textContent = formatElapsed(0);
      state.timerId = window.setInterval(() => {
        els.timer.textContent = formatElapsed(Date.now() - state.startedAt);
      }, 250);
      startWatching();
      setStatus(t("status.recording", { name: splitName(s).filter(Boolean).join(" — ") }));
    } else {
      window.clearInterval(state.timerId);
      stopWatching();
      setStatus(t("status.savingFile"));
      const duration = els.timer.textContent;
      const path = await invoke<string>("stop_recording");
      state.recording = false;
      state.lastPath = path;
      els.lastName.textContent = path.split(/[\\/]/).pop() ?? path;
      els.lastMeta.textContent = `${t("last.duration")} ${duration}`;
      els.last.hidden = false;
      setStatus(t("status.saved"));
    }
  } catch (e) {
    window.clearInterval(state.timerId);
    stopWatching();
    state.recording = false;
    setStatus(String(e), true);
  } finally {
    state.busy = false;
    updateControls();
  }
}

// ---------- Inicio ----------

window.addEventListener("DOMContentLoaded", async () => {
  setLanguage(await invoke<string>("get_language").catch(() => navigator.language.slice(0, 2)));
  translatePage();
  applyTheme(readTheme());
  els.themeButtons.forEach((b) => b.addEventListener("click", () => applyTheme(b.dataset.theme as Theme)));

  els.folds.forEach((fold) =>
    fold.querySelector(".fold-toggle")!.addEventListener("click", () => {
      openSection(fold.classList.contains("open") ? null : fold.dataset.section!);
    }),
  );
  // Al abrir la app, la primera sección queda abierta.
  openSection(els.folds[0]?.dataset.section ?? null);

  els.tabs.forEach((tab) =>
    tab.addEventListener("click", () => {
      state.kind = tab.dataset.kind as SourceKind;
      els.tabs.forEach((t) => t.classList.toggle("active", t === tab));
      renderSources();
    }),
  );
  els.fps.querySelectorAll<HTMLButtonElement>("button").forEach((b) =>
    b.addEventListener("click", () => {
      state.fps = Number(b.dataset.fps);
      els.fps.querySelectorAll("button").forEach((x) => x.classList.toggle("active", x === b));
      updateSummaries();
    }),
  );
  for (const el of [els.systemAudio, els.microphone]) el.addEventListener("change", updateControls);
  els.refresh.addEventListener("click", loadSources);
  els.record.addEventListener("click", toggleRecording);
  els.changeDir.addEventListener("click", changeOutputDir);
  els.alertReal.addEventListener("click", () => {
    state.watchdog?.confirm(state.lastHealth?.elapsedSecs ?? 0);
    showAlert([]);
  });
  els.alertStop.addEventListener("click", () => {
    showAlert([]);
    if (state.recording && !state.busy) toggleRecording();
  });
  els.reveal.addEventListener("click", () => state.lastPath && !preview && revealItemInDir(state.lastPath));

  showOutputDir(await invoke<string>("get_output_dir"));
  await Promise.all([loadSources(), loadMicrophones()]);

  // Cada cambio de tamaño del contenido (secciones que se abren o pliegan,
  // mensajes, la tarjeta de la última grabación) reajusta la ventana.
  const observer = new ResizeObserver(fitWindow);
  for (const el of [...document.querySelector("main")!.children, $(".dock")]) observer.observe(el);
  fitWindow();
});
