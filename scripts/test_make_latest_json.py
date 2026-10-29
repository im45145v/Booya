import json, os, subprocess, sys, tempfile, unittest
sys.path.insert(0, os.path.dirname(__file__))
import importlib
import importlib.util
spec = importlib.util.spec_from_file_location("mk", os.path.join(os.path.dirname(__file__), "make-latest-json.py"))
mk = importlib.util.module_from_spec(spec); spec.loader.exec_module(mk)


class T(unittest.TestCase):
    def test_lists_only_signed_platforms(self):
        with tempfile.TemporaryDirectory() as d:
            for n, s in [("Booya.app.tar.gz", "sigmac"), ("Booya_0.5.0_amd64.AppImage", "siglin"), ("Booya_0.5.0_x64-setup.exe", None), ("x.msi", "ignored")]:
                open(os.path.join(d, n), "w").write("x")
                if s:
                    open(os.path.join(d, n + ".sig"), "w").write(s + "\n")
            doc = mk.build(d, "v0.5.0", "o/r", "notes")
            self.assertEqual(doc["version"], "0.5.0")
            self.assertEqual(sorted(doc["platforms"]), ["darwin-aarch64", "linux-x86_64"])
            self.assertEqual(doc["platforms"]["darwin-aarch64"]["signature"], "sigmac")
            self.assertEqual(doc["platforms"]["linux-x86_64"]["url"], "https://github.com/o/r/releases/download/v0.5.0/Booya_0.5.0_amd64.AppImage")
            json.dumps(doc)

    def test_nothing_signed_gives_none(self):
        with tempfile.TemporaryDirectory() as d:
            open(os.path.join(d, "Booya.app.tar.gz"), "w").write("x")
            self.assertIsNone(mk.build(d, "v0.5.0", "o/r"))


if __name__ == "__main__":
    unittest.main()
