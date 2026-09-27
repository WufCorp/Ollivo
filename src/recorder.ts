// Запись с микрофона для диктовки: сразу 16 кГц, моно — так её ждёт whisper.
// MediaRecorder не годится: он пишет webm/opus, а whisper.cpp читает только wav, mp3, ogg, flac.

// no-inline: маленький файл Vite иначе вклеил бы как data:-адрес, а его CSP не пропустит.
import tapUrl from "./tap-worklet.js?url&no-inline";

const RATE = 16000;

export interface Recording {
  /** Сколько секунд уже записано. */
  seconds(): number;
  /** Останавливает запись и отдаёт WAV в base64. */
  stop(): Promise<string>;
  /** Останавливает без результата. */
  cancel(): void;
}

/** Начинает запись. Ошибка — нет микрофона или доступ к нему запрещён. */
export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
  });
  // Частоту 16 кГц задаём самому AudioContext — пересчёт делает браузер.
  const ctx = new AudioContext({ sampleRate: RATE });
  await ctx.audioWorklet.addModule(tapUrl);
  const source = ctx.createMediaStreamSource(stream);
  const tap = new AudioWorkletNode(ctx, "tap");
  const chunks: Float32Array[] = [];
  let length = 0;
  tap.port.onmessage = (e) => {
    chunks.push(e.data);
    length += e.data.length;
  };
  source.connect(tap);

  const close = () => {
    source.disconnect();
    tap.disconnect();
    stream.getTracks().forEach((t) => t.stop());
    ctx.close();
  };

  return {
    seconds: () => length / RATE,
    cancel: close,
    async stop() {
      close();
      return toBase64(wav(chunks, length));
    },
  };
}

/** PCM 16 бит в обёртке WAV. */
function wav(chunks: Float32Array[], length: number): Uint8Array {
  const buf = new ArrayBuffer(44 + length * 2);
  const v = new DataView(buf);
  const text = (at: number, s: string) => [...s].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  v.setUint32(4, 36 + length * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 1, true); // моно
  v.setUint32(24, RATE, true);
  v.setUint32(28, RATE * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  text(36, "data");
  v.setUint32(40, length * 2, true);
  let at = 44;
  for (const c of chunks) {
    for (let i = 0; i < c.length; i++, at += 2) {
      const s = Math.max(-1, Math.min(1, c[i]));
      v.setInt16(at, s < 0 ? s * 0x8000 : s * 0x7fff, true);
    }
  }
  return new Uint8Array(buf);
}

function toBase64(bytes: Uint8Array): string {
  let s = "";
  // Кусками: String.fromCharCode с миллионом аргументов переполнит стек.
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}
