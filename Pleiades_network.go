package main

import (
	"context"
	"fmt"
	"time"

	"github.com/libp2p/go-libp2p/core/host"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/discovery/mdns"
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
	if _, ok := seen[pi.ID]; ok {
		return
	}
	seen[pi.ID] = struct{}{}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	n.h.Connect(ctx, pi)

	fmt.Println("----------------------------------")
	fmt.Printf("当前网络节点数：%d\n", len(seen)+1) // +1 是自己

	for id := range seen {
		fmt.Println("节点 :", id)
	}
}
