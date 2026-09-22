# repo: https://github.com/Aider-AI/aider (Apache-2.0)
# commit: 5dc9490bb35f9729ef2c95d00a19ccd30c26339c  committed: 2026-05-22T14:02:20Z  retrieved: 2026-09-22
# path: aider/analytics.py  lines 137-145
    def get_data_file_path(self):
        try:
            data_file = Path.home() / ".aider" / "analytics.json"
            data_file.parent.mkdir(parents=True, exist_ok=True)
            return data_file
        except OSError:
            # If we can't create/access the directory, just disable analytics
            self.disable(permanently=False)
            return None
