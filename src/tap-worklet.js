// Обработчик звука для диктовки: живёт в отдельном потоке браузера и отдаёт куски в основной.
// Отдельным файлом, а не blob:-адресом: CSP окна разрешает скрипты только из самой программы.
class Tap extends AudioWorkletProcessor {
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch) this.port.postMessage(ch.slice(0));
    return true;
  }
}
registerProcessor("tap", Tap);
