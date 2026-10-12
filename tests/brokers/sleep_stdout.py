#!/usr/bin/env python3
import os, sys, time
if len(sys.argv) > 1 and sys.argv[1] == "--version":
    sys.exit(0)
time.sleep(float(os.environ.get("CM_BROKER_SLEEP", "30")))
