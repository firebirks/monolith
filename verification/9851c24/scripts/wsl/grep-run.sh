#!/bin/bash
cd ~/p8 && grep -n 'survived\|invalid\|timed out\|timeout\|caught' mutation/run.py | head -30
