"""Rebuild the whole Booya brand kit. Run from anywhere: python3 src/build_all.py"""
import os, runpy, sys
here = os.path.dirname(os.path.abspath(__file__))
os.chdir(here); sys.path.insert(0, here)
for m in ['build_logo', 'build_icons', 'build_mascot', 'build_tokens', 'build_social', 'guideline', 'build_overview']:
    print('building', m)
    runpy.run_path(os.path.join(here, m + '.py'), run_name='__main__')
