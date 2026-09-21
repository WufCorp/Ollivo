"""Генерация картинки через API ComfyUI (без веб-интерфейса) с замером времени и видеопамяти."""
import json, subprocess, sys, threading, time, urllib.request, uuid

HOST = "http://127.0.0.1:8188"
ckpt = sys.argv[1] if len(sys.argv) > 1 else "v1-5-pruned-emaonly-fp16.safetensors"
size = int(sys.argv[2]) if len(sys.argv) > 2 else 512
prompt_text = sys.argv[3] if len(sys.argv) > 3 else "a cozy wooden cabin in a snowy forest at sunset, detailed, warm light"

wf = {
  "1": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": ckpt}},
  "2": {"class_type": "CLIPTextEncode", "inputs": {"clip": ["1", 1], "text": prompt_text}},
  "3": {"class_type": "CLIPTextEncode", "inputs": {"clip": ["1", 1], "text": "blurry, low quality"}},
  "4": {"class_type": "EmptyLatentImage", "inputs": {"width": size, "height": size, "batch_size": 1}},
  "5": {"class_type": "KSampler", "inputs": {"model": ["1", 0], "positive": ["2", 0], "negative": ["3", 0],
        "latent_image": ["4", 0], "seed": int(time.time()), "steps": 20, "cfg": 7.0, "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0}},
  "6": {"class_type": "VAEDecode", "inputs": {"samples": ["5", 0], "vae": ["1", 2]}},
  "7": {"class_type": "SaveImage", "inputs": {"images": ["6", 0], "filename_prefix": "probe"}},
}

peak = [0]
stop = False
def watch_vram():
    while not stop:
        out = subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], capture_output=True, text=True).stdout
        if out.strip().isdigit():  # под нагрузкой nvidia-smi иногда не отвечает
            peak[0] = max(peak[0], int(out.strip()))
        time.sleep(0.25)

base = int(subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], capture_output=True, text=True).stdout)
threading.Thread(target=watch_vram, daemon=True).start()

body = json.dumps({"prompt": wf, "client_id": str(uuid.uuid4())}).encode()
t0 = time.time()
pid = json.load(urllib.request.urlopen(urllib.request.Request(HOST + "/prompt", body, {"Content-Type": "application/json"})))["prompt_id"]
while True:
    h = json.load(urllib.request.urlopen(f"{HOST}/history/{pid}"))
    if pid in h:
        break
    time.sleep(0.3)
dt = time.time() - t0
stop = True
st = h[pid]["status"]
imgs = [i["filename"] for o in h[pid]["outputs"].values() for i in o.get("images", [])]
print(f"статус: {st.get('status_str')}, картинки: {imgs}")
print(f"время: {dt:.1f} с; видеопамять: было {base} МиБ, пик {peak[0]} МиБ (+{peak[0]-base})")
