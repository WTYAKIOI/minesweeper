#!/bin/env python3
# SPDX-FileCopyrightText: 2017 Enantiomerie
# SPDX-License-Identifier: MIT

"""Example OCRmyPDF for Synology NAS."""

from __future__ import annotations

# This script must be edited to meet your needs.
import logging
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

# pylint: disable=logging-format-interpolation
# pylint: disable=logging-not-lazy

script_dir = Path(os.path.realpath(__file__)).parent
timestamp = time.strftime("%Y-%m-%d-%H%M_")
log_file = script_dir / (timestamp + 'ocrmypdf.log')
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s %(message)s',
    filename=log_file,
    filemode='w',
)

start_dir = sys.argv[1] if len(sys.argv) > 1 else '.'

for dir_name, _subdirs, file_list in os.walk(start_dir):
    logging.info(dir_name)
    os.chdir(dir_name)
    for filename in file_list:
        file_stem, file_ext = Path(filename).stem, Path(filename).suffix
        if file_ext != '.pdf':
            continue
        full_path = Path(dir_name, filename)
        timestamp_ocr = time.strftime("%Y-%m-%d-%H%M_OCR_")
        filename_ocr = timestamp_ocr + file_stem + '.pdf'
        # create string for pdf processing
        # the script is processed as root user via chron
        cmd = [
            'docker',
            'run',
            '--rm',
            '-i',
            'jbarlow83/ocrmypdf',
            '--deskew',
            '-',
            '-',
        ]
        logging.info(cmd)
        full_path_ocr = Path(dir_name, filename_ocr)
        with (
            Path(filename).open('rb') as input_file,
            full_path_ocr.open('wb') as output_file,
        ):
            proc = subprocess.run(
                cmd,
                stdin=input_file,
                stdout=output_file,
                stderr=subprocess.PIPE,
                check=False,
                text=True,
                errors='ignore',
            )
        logging.info(proc.stderr)
        full_path_ocr.chmod(0o664)
        full_path.chmod(0o664)
        full_path_ocr_archive = sys.argv[2]
        full_path_archive = sys.argv[2] + '/no_ocr'
        shutil.move(full_path_ocr, full_path_ocr_archive)
        shutil.move(full_path, full_path_archive)
logging.info('Finished.\n')
