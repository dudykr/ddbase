// The companion to compare.rs. Build this file explicitly; no Go module or
// external dependencies are needed. Blocking syscall mode targets Unix.
package main

import (
	"flag"
	"fmt"
	"math/bits"
	"net"
	"os"
	"runtime"
	"sort"
	"syscall"
	"time"
)

type context struct {
	file     *os.File
	fd       int
	checksum uint64
}

func check(err error) {
	if err != nil {
		panic(err)
	}
}

//go:noinline
func cpu(value uint64) uint64 {
	for i := 0; i < 10_000; i++ {
		value = bits.RotateLeft64(value*6364136223846793005, 7)
	}
	return value
}

func operation(workload string, iteration int) string {
	if workload == "Mixed" {
		switch iteration % 4 {
		case 0:
			return "SyscallWait"
		case 1:
			return "CachedRead"
		default:
			return "Cpu"
		}
	}
	return workload
}

func (c *context) run(op, path string) {
	switch op {
	case "Yield", "Empty":
	case "Cpu":
		c.checksum ^= cpu(17)
	case "CachedRead":
		data, err := os.ReadFile(path)
		check(err)
		if len(data) != 4096 {
			panic("expected a 4 KiB fixture")
		}
		c.checksum ^= uint64(data[0])
	case "SyscallWait":
		// syscall.Read/Write enter the Go runtime's syscall state. RawSyscall
		// would bypass the scheduler; net.Conn.Read would use the netpoller.
		byte := []byte{1}
		for {
			n, err := syscall.Write(c.fd, byte)
			if err == syscall.EINTR {
				continue
			}
			check(err)
			if n != 1 {
				panic("short socket write")
			}
			break
		}
		for {
			n, err := syscall.Read(c.fd, byte)
			if err == syscall.EINTR {
				continue
			}
			check(err)
			if n != 1 || byte[0] != 1 {
				panic(fmt.Sprintf("invalid socket reply: fd=%d n=%d byte=%d", c.fd, n, byte[0]))
			}
			break
		}
	default:
		panic("unknown case: " + op)
	}
}

func contexts(workload, peer string, lanes int) []context {
	result := make([]context, lanes)
	for i := range result {
		if workload != "SyscallWait" && workload != "Mixed" {
			continue
		}
		conn, err := net.DialTimeout("tcp", peer, 30*time.Second)
		check(err)
		tcp := conn.(*net.TCPConn)
		check(tcp.SetNoDelay(true))
		file, err := tcp.File()
		check(err)
		check(conn.Close())
		fd := int(file.Fd())
		check(syscall.SetNonblock(fd, false))
		timeout := syscall.NsecToTimeval(int64(30 * time.Second))
		check(syscall.SetsockoptTimeval(fd, syscall.SOL_SOCKET, syscall.SO_RCVTIMEO, &timeout))
		result[i] = context{file: file, fd: fd}
		// Finish accepting/setup outside timing, using the same delayed peer.
		result[i].run("SyscallWait", "")
	}
	return result
}

func run(workload, path string, contexts []context, iterations int, latency bool) []time.Duration {
	completed := make(chan []time.Duration, len(contexts))
	for _, c := range contexts {
		go func(c context) {
			var samples []time.Duration
			if latency {
				samples = make([]time.Duration, 0, iterations)
			}
			for i := 0; i < iterations; i++ {
				op := operation(workload, i)
				if latency {
					start := time.Now()
					c.run(op, path)
					samples = append(samples, time.Since(start))
				} else {
					c.run(op, path)
				}
				runtime.Gosched()
			}
			runtime.KeepAlive(c)
			if c.file != nil {
				check(c.file.Close())
			}
			completed <- samples
		}(c)
	}
	var samples []time.Duration
	for range contexts {
		samples = append(samples, <-completed...)
	}
	return samples
}

func main() {
	workload := flag.String("case", "Empty", "workload selected by compare.rs")
	parallelism := flag.Int("parallelism", 4, "GOMAXPROCS")
	lanes := flag.Int("lanes", 16, "concurrent goroutines")
	iterations := flag.Int("iterations", 2000, "operations per lane")
	path := flag.String("path", "", "4 KiB file fixture")
	peer := flag.String("peer", "", "external delayed echo server")
	latency := flag.Bool("latency", true, "collect per-operation latency")
	flag.Parse()
	if *parallelism <= 0 || *lanes <= 0 || *iterations <= 0 {
		panic("parallelism, lanes and iterations must be positive")
	}
	runtime.GOMAXPROCS(*parallelism)
	fmt.Fprintf(os.Stderr, "Go %s; GOMAXPROCS=%d; blocking syscalls; default GC; thread cap not imposed\n", runtime.Version(), *parallelism)
	warm := contexts(*workload, *peer, *lanes)
	run(*workload, *path, warm, min(*iterations, 32), *latency)
	measured := contexts(*workload, *peer, *lanes)
	start := time.Now()
	samples := run(*workload, *path, measured, *iterations, *latency)
	elapsed := time.Since(start)
	sort.Slice(samples, func(i, j int) bool { return samples[i] < samples[j] })
	quantile := func(percent int) string {
		if len(samples) == 0 {
			return ""
		}
		return fmt.Sprint(samples[(len(samples)*percent+99)/100-1].Nanoseconds())
	}
	count := *lanes * *iterations
	// Process startup is excluded. Go exposes no comparable worker-only peak
	// count or handoff counter, so init/worker/handoff values are not fabricated.
	fmt.Printf("Go,%s,steady,%d,%d,%.2f,%s,%s,,\n", *workload, count, elapsed.Microseconds(), float64(count)/elapsed.Seconds(), quantile(50), quantile(99))
}
