import { spawnSync } from 'node:child_process';

const npmCommand = process.platform === 'win32' ? 'npm.cmd' : 'npm';

function run(args, env) {
  const result = spawnSync(npmCommand, args, {
    env,
    shell: process.platform === 'win32',
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

for (const platform of ['macos', 'windows']) {
  run(['run', 'build'], {
    ...process.env,
    VITE_POSTHOG_KEY: 'phc_toolbox_e2e_flags',
    VITE_TOOLBOX_TARGET_PLATFORM: platform,
  });
  run(['exec', '--', 'playwright', 'test', 'tests/ui/pdf-scene.spec.ts', '--grep=PostHog'], {
    ...process.env,
    TOOLBOX_EXPECTED_PLATFORM: platform,
    TOOLBOX_UI_PRODUCTION: '1',
  });
}
