// Vigila que la grabación no esté quedando vacía y pregunta a la persona:
//
// - Imagen negra: solo durante el primer minuto y como máximo 2 veces. Si sale
//   negra 5 s seguidos, pregunta; si el negro es real y sigue, vuelve a
//   preguntar 30 s después (o en cuanto vuelva a ponerse negra).
// - Micrófono: si a los 5 s no llegó nada, pregunta; si el silencio es real,
//   vuelve a preguntar cada 30 s mientras siga así, hasta los 5 minutos.
// - Audio del sistema: pregunta a los 5 s y, si sigue sin llegar nada, una vez
//   más a los 2 minutos (que no suene nada al principio es normal).
//
// - Sonido perdido: si una fuente que ya tenía sonido pasa 15 s en silencio
//   absoluto (en cualquier momento), avisa una vez por cada corte.
//
// La grabación nunca se detiene sola: solo si la persona pulsa «Detener».

export type Check = "black" | "microphone" | "system" | "microphoneLost" | "systemLost";

export interface HealthReport {
  elapsedSecs: number;
  systemHeard: boolean;
  systemSilentSecs: number;
  microphoneHeard: boolean;
  microphoneSilentSecs: number;
  blackSecs: number;
}

const FIRST_CHECK_SECS = 5;
const BLACK_WINDOW_SECS = 60;
const BLACK_MAX_ASKS = 2;
const BLACK_REPEAT_SECS = 30;
const MIC_REPEAT_SECS = 30;
const MIC_WINDOW_SECS = 5 * 60;
const SYSTEM_SECOND_ASK_SECS = 2 * 60;
const LOST_AFTER_SECS = 15;
/** Por debajo de esto se considera que el sonido volvió. */
const SOUND_BACK_SECS = 1;

export class Watchdog {
  private nextMicAsk = FIRST_CHECK_SECS;
  private nextSystemAsk = FIRST_CHECK_SECS;
  private systemAsks = 0;
  private blackAsks = 0;
  private nextBlackAsk = 0;
  /** Ya se avisó del corte actual de cada fuente; se rearma cuando vuelve el sonido. */
  private lostWarned = { microphone: false, system: false };
  /** Preguntas abiertas: si fallan varias cosas a la vez, se preguntan juntas. */
  private open = new Set<Check>();

  /** `sources`: qué fuentes de audio vigilar (las grabadas, si se pidió avisar). */
  constructor(private readonly sources: { system: boolean; microphone: boolean }) {}

  get questions(): Check[] {
    return [...this.open];
  }

  /** Revisa el estado; devuelve las preguntas que hay que mostrar (vacío si ninguna). */
  update(r: HealthReport): Check[] {
    // Si la imagen volvió, un negro nuevo se puede preguntar enseguida.
    if (r.blackSecs === 0) this.nextBlackAsk = 0;

    const micBack = r.microphoneHeard && r.microphoneSilentSecs < SOUND_BACK_SECS;
    const systemBack = r.systemHeard && r.systemSilentSecs < SOUND_BACK_SECS;
    if (micBack) this.lostWarned.microphone = false;
    if (systemBack) this.lostWarned.system = false;

    // Si mientras se preguntaba volvió la señal, esa pregunta sobra.
    const resolved: Record<Check, boolean> = {
      black: r.blackSecs === 0,
      microphone: r.microphoneHeard,
      system: r.systemHeard,
      microphoneLost: micBack,
      systemLost: systemBack,
    };
    for (const check of this.open) if (resolved[check]) this.open.delete(check);

    const t = r.elapsedSecs;
    const due: Record<Check, boolean> = {
      black:
        t <= BLACK_WINDOW_SECS + FIRST_CHECK_SECS &&
        this.blackAsks < BLACK_MAX_ASKS &&
        t >= this.nextBlackAsk &&
        r.blackSecs >= FIRST_CHECK_SECS,
      microphone:
        this.sources.microphone &&
        !r.microphoneHeard &&
        t >= this.nextMicAsk &&
        t <= MIC_WINDOW_SECS &&
        r.microphoneSilentSecs >= FIRST_CHECK_SECS,
      system:
        this.sources.system &&
        !r.systemHeard &&
        this.systemAsks < 2 &&
        t >= this.nextSystemAsk &&
        r.systemSilentSecs >= FIRST_CHECK_SECS,
      microphoneLost:
        this.sources.microphone &&
        r.microphoneHeard &&
        !this.lostWarned.microphone &&
        r.microphoneSilentSecs >= LOST_AFTER_SECS,
      systemLost:
        this.sources.system && r.systemHeard && !this.lostWarned.system && r.systemSilentSecs >= LOST_AFTER_SECS,
    };
    for (const check of Object.keys(due) as Check[]) if (due[check]) this.open.add(check);
    return this.questions;
  }

  /** La persona confirmó que todo lo preguntado es real: seguir grabando. */
  confirm(elapsedSecs: number) {
    for (const check of this.open) {
      switch (check) {
        case "black":
          this.blackAsks += 1;
          this.nextBlackAsk = elapsedSecs + BLACK_REPEAT_SECS;
          break;
        case "microphone":
          this.nextMicAsk = elapsedSecs + MIC_REPEAT_SECS;
          break;
        case "system":
          this.systemAsks += 1;
          this.nextSystemAsk = Math.max(elapsedSecs + 1, SYSTEM_SECOND_ASK_SECS);
          break;
        case "microphoneLost":
          this.lostWarned.microphone = true;
          break;
        case "systemLost":
          this.lostWarned.system = true;
          break;
      }
    }
    this.open.clear();
  }
}
