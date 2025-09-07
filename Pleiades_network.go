package main

import (
	"context"
	"fmt"
	"time"

	"github.com/libp2p/go-libp2p/core/host"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/discovery/mdns"
	"github.com/libp2p/go-libp2p/p2p/protocol/ping"
)

func Setup_MDNS(h host.Host) error {
	// NewMdnsService 默认会把本节点也注册到 mDNS 里
	// 根据MDNS的设置，每1秒发送一次mDNS广播
	svc := mdns.NewMdnsService(h, SERVICE_TAG, &notifee{h})
	if err := svc.Start(); err != nil {
		panic(err)
	}
	return nil
}

func (n *notifee) HandlePeerFound(pi peer.AddrInfo) {
	seenMu.Lock()
	if _, ok := seen[pi.ID]; ok {
		seenMu.Unlock()
		return
	}
	seenMu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := n.h.Connect(ctx, pi); err != nil {
		fmt.Println("mdns 连接失败:", err)
		return
	}

	seenMu.Lock()
	seen[pi.ID] = struct{}{}
	seenMu.Unlock()

	fmt.Println("----------------------------------")
	Print_Peers()
}

func Check_Connection(h host.Host) {
	pingSvc := ping.NewPingService(h)
	ticker := time.NewTicker(10 * time.Second) // 每 10 s 扫一轮
	defer ticker.Stop()

	for range ticker.C {
		// 拷贝当前列表
		seenMu.RLock()
		ids := make([]peer.ID, 0, len(seen))
		for id := range seen {
			ids = append(ids, id)
		}
		seenMu.RUnlock()

		// 逐个 ping
		for _, id := range ids {
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			resCh := pingSvc.Ping(ctx, id)
			select {
			case <-resCh:
				// 通了，什么也不做
			case <-ctx.Done():
				// 超时：删表 + 关闭连接
				fmt.Printf("集中心跳超时，移除节点: %s\n", id)
				h.Network().ClosePeer(id)
				seenMu.Lock()
				delete(seen, id)
				seenMu.Unlock()
			}
			cancel()
		}

		// 可选：每轮结束后打印一次
		Print_Peers()
	}
}
