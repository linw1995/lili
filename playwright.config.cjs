const { defineConfig } = require("@playwright/test");

module.exports = defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  use: {
    baseURL: "http://127.0.0.1:3746",
    trace: "retain-on-failure",
  },
  webServer: {
    command:
      "trunk build --locked --config Trunk.web.toml && cargo run --locked --package lili-web",
    url: "http://127.0.0.1:3746/health",
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
