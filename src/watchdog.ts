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
// En cuanto llega sonido de una fuente, no vuelve a preguntar por ella. La
// grabación nunca se detiene sola: solo si la persona pulsa «Detener».

export type Check = "black" | "microphone" | "system";

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

export class Watchdog {
  private nextMicAsk = FIRST_CHECK_SECS;
  private nextSystemAsk = FIRST_CHECK_SECS;
  private systemAsks = 0;
  private blackAsks = 0;
  private nextBlackAsk = 0;
  private open: Check | null = null;

  constructor(private readonly sources: { system: boolean; microphone: boolean }) {}

  get question(): Check | null {
    return this.open;
  }

  /** Revisa el estado; devuelve la pregunta que hay que mostrar, o null. */
  update(r: HealthReport): Check | null {
    // Si la imagen volvió, un negro nuevo se puede preguntar enseguida.
    if (r.blackSecs === 0) this.nextBlackAsk = 0;

    // Si mientras se preguntaba volvió la señal, la pregunta sobra.
    if (this.open === "black" && r.blackSecs === 0) this.open = null;
    if (this.open === "microphone" && r.microphoneHeard) this.open = null;
    if (this.open === "system" && r.systemHeard) this.open = null;
    if (this.open) return this.open;

    const t = r.elapsedSecs;
    if (
      t <= BLACK_WINDOW_SECS + FIRST_CHECK_SECS &&
      this.blackAsks < BLACK_MAX_ASKS &&
      t >= this.nextBlackAsk &&
      r.blackSecs >= FIRST_CHECK_SECS
    ) {
      this.open = "black";
    } else if (
      this.sources.microphone &&
      !r.microphoneHeard &&
      t >= this.nextMicAsk &&
      t <= MIC_WINDOW_SECS &&
      r.microphoneSilentSecs >= FIRST_CHECK_SECS
    ) {
      this.open = "microphone";
    } else if (
      this.sources.system &&
      !r.systemHeard &&
      this.systemAsks < 2 &&
      t >= this.nextSystemAsk &&
      r.systemSilentSecs >= FIRST_CHECK_SECS
    ) {
      this.open = "system";
    }
    return this.open;
  }

  /** La persona confirmó que el silencio (o el negro) es real: seguir grabando. */
  confirm(elapsedSecs: number) {
    switch (this.open) {
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
    }
    this.open = null;
  }
}
