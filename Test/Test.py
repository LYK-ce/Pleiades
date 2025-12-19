#Presented by KeJi
#Date : 2025-12-16

import sys
import os
import argparse

# 添加当前目录到路径
sys.path.insert(0, os.path.dirname(__file__))


def Run_Runtime_Tests():
    """运行Runtime测试"""
    print("=" * 60)
    print("运行Runtime层测试")
    print("=" * 60)
    
    import Runtime_Test
    result = Runtime_Test.Run_All_Tests()
    
    return result


def Create_Models():
    """创建并保存模型"""
    print("=" * 60)
    print("创建和保存模型")
    print("=" * 60)
    
    # 创建CNN模型
    print("\n创建CNN模型...")
    from Model import CNN_Model
    CNN_Model.Create_And_Save_CNN_Model()
    
    # 创建ViT模型
    print("\n创建ViT模型...")
    from Model import ViT_Model
    ViT_Model.Create_And_Save_ViT_Model()
    
    print("\n所有模型创建完成!")
    return True


def Run_All_Tests():
    """运行所有测试"""
    print("\n" + "=" * 60)
    print("运行所有测试套件")
    print("=" * 60 + "\n")
    
    results = {}
    
    # 运行Runtime测试
    results['runtime'] = Run_Runtime_Tests()
    
    # 打印汇总
    print("\n" + "=" * 60)
    print("测试汇总")
    print("=" * 60)
    for test_name, success in results.items():
        status = "✓ 通过" if success else "✗ 失败"
        print(f"{test_name.upper()}: {status}")
    
    all_passed = all(results.values())
    print("\n总体结果:", "✓ 所有测试通过" if all_passed else "✗ 存在失败的测试")
    
    return all_passed


def List_Available_Tests():
    """列出所有可用的测试"""
    print("\n可用的测试选项:")
    print("  runtime    - 运行Runtime层测试")
    print("  models     - 创建和保存模型")
    print("  all        - 运行所有测试")
    print("  list       - 列出所有可用测试")


def Main():
    """主函数：解析命令行参数并执行相应的测试"""
    parser = argparse.ArgumentParser(
        description='Pleiades测试框架 - 统一测试入口',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  python Test.py runtime        运行Runtime层测试
  python Test.py models          创建并保存模型
  python Test.py all             运行所有测试
  python Test.py list            列出所有可用测试
        """
    )
    
    parser.add_argument(
        'test_target',
        nargs='?',
        default='all',
        choices=['runtime', 'models', 'all', 'list'],
        help='要运行的测试目标 (默认: all)'
    )
    
    args = parser.parse_args()
    
    # 根据参数执行相应的测试
    if args.test_target == 'runtime':
        success = Run_Runtime_Tests()
        sys.exit(0 if success else 1)
    
    elif args.test_target == 'models':
        success = Create_Models()
        sys.exit(0 if success else 1)
    
    elif args.test_target == 'all':
        success = Run_All_Tests()
        sys.exit(0 if success else 1)
    
    elif args.test_target == 'list':
        List_Available_Tests()
        sys.exit(0)
    
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    Main()
