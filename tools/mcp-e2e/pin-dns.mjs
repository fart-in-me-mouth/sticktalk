// Preloaded with `node --import` by run.ps1 -PublicPath: resolve E2E_PIN_HOST to
// E2E_PIN_IP (the Funnel ingress from public DNS), so tests run on the host
// machine still travel the public internet path instead of the tailnet.
import dns from "node:dns";

const host = process.env.E2E_PIN_HOST;
const ip = process.env.E2E_PIN_IP;
const original = dns.lookup;
dns.lookup = function lookup(name, options, callback) {
  if (typeof options === "function") [callback, options] = [options, {}];
  if (name !== host) return original.call(dns, name, options, callback);
  const family = ip.includes(":") ? 6 : 4;
  if (options && options.all) return process.nextTick(callback, null, [{ address: ip, family }]);
  return process.nextTick(callback, null, ip, family);
};
