import json, sys, time, urllib.request
port = sys.argv[1]; q = sys.argv[2] if len(sys.argv) > 2 else "Привет! Кратко: что такое фотосинтез?"
body = json.dumps({"messages":[{"role":"user","content":q}],"stream":True,"max_tokens":200}).encode()
req = urllib.request.Request(f"http://localhost:{port}/v1/chat/completions", body, {"Content-Type":"application/json"})
t0 = time.time(); first = None; n = 0; text = ""; timings = None
with urllib.request.urlopen(req) as r:
    for line in r:
        line = line.decode("utf-8").strip()
        if not line.startswith("data: {"): continue
        d = json.loads(line[6:])
        if d.get("timings"): timings = d["timings"]
        c = (d.get("choices") or [{}])[0].get("delta", {}).get("content")
        if c:
            first = first or time.time(); n += 1; text += c
print(text)
print(f"--- первый токен через {(first-t0)*1000:.0f} мс, чанков {n}, всего {time.time()-t0:.1f} с")
if timings: print("prompt", round(timings["prompt_per_second"]), "ток/с; генерация", round(timings["predicted_per_second"]), "ток/с")
