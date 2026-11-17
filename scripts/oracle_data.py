"""The data files of apps/oracle/src that the scripts read as the oracle does: the vocabulary of its verdicts
(vocabulary.json, written from vocabulary.ts), the shape rules (component-shape.json) and the path classes
(path-classes.json). scripts/test_oracle_data.py, test_component_shape.py and test_path_classes.py run the oracle's cases
against them, so a rule changed on one side only fails a test."""
import json
import os
import re

ORACLE_SRC = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'apps', 'oracle', 'src')


def read(name):
    with open(os.path.join(ORACLE_SRC, name)) as f:
        return json.load(f)


VOCABULARY = read('vocabulary.json')
SHAPE = read('component-shape.json')
PATHS = read('path-classes.json')

VERDICTS = VOCABULARY['verdicts']
AGREE = VOCABULARY['agree']

DEPENDENCY = re.compile(PATHS['dependency'])
REACT_TYPES = re.compile(PATHS['reactTypes'])
LIB_FILE = re.compile(PATHS['libFile'])


def _slashed(path):
    return path.replace('\\', '/')


def is_dependency(path):
    """Whether the file is installed under a `node_modules` directory, `@types/react` and the lib files included."""
    return bool(DEPENDENCY.search(_slashed(path)))


def is_react_types(path):
    """Whether the file is part of `@types/react`."""
    return bool(REACT_TYPES.search(_slashed(path)))


def is_lib_file(path):
    """Whether the file is one of TypeScript's `lib.*.d.ts`."""
    return bool(LIB_FILE.search(_slashed(path)))
