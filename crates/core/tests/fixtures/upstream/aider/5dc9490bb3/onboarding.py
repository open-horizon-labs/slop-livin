# repo: https://github.com/Aider-AI/aider (Apache-2.0)
# commit: 5dc9490bb35f9729ef2c95d00a19ccd30c26339c  committed: 2026-05-22T14:02:20Z  retrieved: 2026-09-22
# path: aider/onboarding.py  lines 360-368

        # Save the key to the oauth-keys.env file
        try:
            config_dir = os.path.expanduser("~/.aider")
            os.makedirs(config_dir, exist_ok=True)
            key_file = os.path.join(config_dir, "oauth-keys.env")
            with open(key_file, "a", encoding="utf-8") as f:
                f.write(f'OPENROUTER_API_KEY="{api_key}"\n')

