#!/bin/bash
# Pauses or resumes the mutation baseline: every process under
# final-mutation.sh, resume-mutation.sh, targeted-mutation.sh or targeted-c.sh (the runner, its cargo runs in their own sessions, the
# compilers and test binaries) gets SIGSTOP, parents first, or SIGCONT,
# children first. The list of processes paused is kept in ~/baseline.paused.
# Usage: pause-baseline.sh stop|cont
set -u
action=$1
tree() {
    local pid=$1 child
    echo "$pid"
    for child in $(ps -o pid= --ppid "$pid"); do
        tree "$child"
    done
}
case $action in
stop)
    root=$(pgrep -f 'final-mutation.sh|resume-mutation.sh|targeted-mutation.sh|targeted-c.sh|repro-unmodified.sh|repro-gatec.sh|stress-restarts.sh|c2-unmodified.sh|targeted-c2.sh|fixcheck.sh|c3-unmodified.sh|targeted-c3.sh|c3r-unmodified.sh|targeted-c3r.sh' | head -1)
    [ -n "$root" ] || { echo "no baseline running"; exit 1; }
    tree "$root" > ~/baseline.paused
    while read -r pid; do kill -STOP "$pid" 2>/dev/null; done < ~/baseline.paused
    # Processes started between the listing and the signals.
    sleep 1
    tree "$root" | sort -u > ~/baseline.paused.2
    sort -u ~/baseline.paused > ~/baseline.paused.1
    comm -13 ~/baseline.paused.1 ~/baseline.paused.2 | while read -r pid; do kill -STOP "$pid" 2>/dev/null; done
    cat ~/baseline.paused.1 ~/baseline.paused.2 | sort -u > ~/baseline.paused
    rm -f ~/baseline.paused.1 ~/baseline.paused.2
    echo "paused $(wc -l < ~/baseline.paused) processes at $(date -Is)"
    ps -o pid,stat,etime,args -p "$(paste -sd, ~/baseline.paused)" | cut -c1-140
    ;;
cont)
    [ -f ~/baseline.paused ] || { echo "nothing paused"; exit 1; }
    tac ~/baseline.paused | while read -r pid; do kill -CONT "$pid" 2>/dev/null; done
    echo "resumed at $(date -Is)"
    ps -o pid,stat,etime,args -p "$(paste -sd, ~/baseline.paused)" 2>/dev/null | cut -c1-140
    rm -f ~/baseline.paused
    ;;
esac
