#Presented by KeJi
#Date : 2025-12-16

import sys
import os
import torch

# 添加Src目录到路径
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(__file__)), 'Src'))

from Runtime import Runtime


def Test_Get_System_Snapshot():
    """测试系统快照功能"""
    print("\n测试 Get_System_Snapshot...")
    runtime = Runtime()
    snapshot = runtime.Get_System_Snapshot()
    
    # 验证返回字典
    assert isinstance(snapshot, dict), "返回值应为字典"
    
    # 验证包含基本信息
    assert 'cpu_percent' in snapshot, "缺少CPU信息"
    assert 'memory_total' in snapshot, "缺少内存信息"
    assert 'network_interfaces' in snapshot, "缺少网络信息"
    assert 'gpu_available' in snapshot, "缺少GPU信息"
    
    print(f"  CPU使用率: {snapshot['cpu_percent']}%")
    print(f"  内存使用率: {snapshot['memory_percent']}%")
    print(f"  GPU可用: {snapshot['gpu_available']}")
    
    # 输出网络信息
    print(f"  网络接口数量: {len(snapshot['network_interfaces'])}")
    print(f"  网络发送字节: {snapshot['network_io']['bytes_sent']:,}")
    print(f"  网络接收字节: {snapshot['network_io']['bytes_recv']:,}")
    print(f"  发送带宽: {snapshot['network_io']['bandwidth_mbps_sent']:.2f} Mbps")
    print(f"  接收带宽: {snapshot['network_io']['bandwidth_mbps_recv']:.2f} Mbps")
    if snapshot.get('network_latency_ms') is not None:
        print(f"  网络延迟: {snapshot['network_latency_ms']:.2f} ms")
    else:
        print(f"  网络延迟: 无法测量")
    
    print("  [OK] Get_System_Snapshot 测试通过")


def Test_Load_Model():
    """测试模型加载功能"""
    print("\n测试 Load_Model...")
    runtime = Runtime()
    weights_dir = os.path.join(os.path.dirname(__file__), 'Weights')
    
    # 测试加载CNN模型
    print("  加载CNN模型...")
    cnn_path = os.path.join(weights_dir, 'cnn_model.pth')
    result = runtime.Load_Model(cnn_path)
    assert result == True, "CNN模型加载失败"
    assert runtime.model is not None, "模型未正确赋值"
    print("    [OK] CNN模型加载成功")
    
    # 测试加载ViT模型
    print("  加载ViT模型...")
    vit_path = os.path.join(weights_dir, 'vit_model.pth')
    result = runtime.Load_Model(vit_path)
    assert result == True, "ViT模型加载失败"
    assert runtime.model is not None, "模型未正确赋值"
    print("    [OK] ViT模型加载成功")
    
    # 测试加载不存在的模型
    print("  测试加载不存在的模型...")
    result = runtime.Load_Model("nonexistent.pth")
    assert result == False, "应该返回False"
    print("    [OK] 正确处理不存在的模型")
    
    print("  [OK] Load_Model 测试通过")


def Test_Execute():
    """测试模型执行功能"""
    print("\n测试 Execute...")
    runtime = Runtime()
    weights_dir = os.path.join(os.path.dirname(__file__), 'Weights')
    
    # 测试未加载模型时执行
    print("  测试未加载模型时执行...")
    try:
        dummy_input = torch.randn(1, 3, 32, 32)
        runtime.Execute(dummy_input)
        assert False, "应该抛出异常"
    except ValueError:
        print("    [OK] 正确抛出异常")
    
    # 测试CNN模型推理
    print("  测试CNN模型推理...")
    cnn_path = os.path.join(weights_dir, 'cnn_model.pth')
    runtime.Load_Model(cnn_path)
    dummy_input = torch.randn(2, 3, 32, 32)
    output = runtime.Execute(dummy_input)
    assert output.shape == (2, 10), f"输出形状错误: {output.shape}"
    print(f"    输出形状: {output.shape} [OK]")
    
    # 测试ViT模型推理
    print("  测试ViT模型推理...")
    vit_path = os.path.join(weights_dir, 'vit_model.pth')
    runtime.Load_Model(vit_path)
    dummy_input = torch.randn(2, 3, 32, 32)
    output = runtime.Execute(dummy_input)
    assert output.shape == (2, 10), f"输出形状错误: {output.shape}"
    print(f"    输出形状: {output.shape} [OK]")
    
    print("  [OK] Execute 测试通过")


def Run_All_Tests():
    """运行所有测试"""
    print("=" * 60)
    print("开始Runtime测试")
    print("=" * 60)
    
    try:
        Test_Get_System_Snapshot()
        Test_Load_Model()
        Test_Execute()
        
        print("\n" + "=" * 60)
        print("[SUCCESS] 所有测试通过")
        print("=" * 60)
        return True
        
    except AssertionError as e:
        print(f"\n[FAIL] 测试失败: {e}")
        return False
    except Exception as e:
        print(f"\n[ERROR] 发生错误: {e}")
        import traceback
        traceback.print_exc()
        return False


if __name__ == "__main__":
    success = Run_All_Tests()
    sys.exit(0 if success else 1)
