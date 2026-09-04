# Level 2: 事件系统

## 订阅事件

在 `_setup_subscriptions()` 中订阅感兴趣的事件：

```python
def _setup_subscriptions(self) -> None:
    from shared_types.events import VideoSaveEvent, BoardUpdateEvent
    
    self.subscribe(VideoSaveEvent, self._on_video_save)
    self.subscribe(BoardUpdateEvent, self._on_board_update)
```

## 可用事件类型

### VideoSaveEvent - 游戏结束

一局游戏结束时触发，包含完整的统计数据：

```python
def _on_video_save(self, event: VideoSaveEvent):
    # 基本信息
    event.rtime           # 用时（秒）
    event.level           # 难度
    event.game_board_state  # 游戏状态（胜利/失败）
    
    # 操作统计
    event.left            # 左键次数
    event.right           # 右键次数
    event.double          # 双击次数
    event.cl              # 总点击数
    event.ce              # 有效点击数
    
    # 3BV 相关
    event.bbbv            # 总 3BV
    event.bbbv_solved     # 已解决 3BV
    
    # 效率指标
    event.stnb            # STNB 分数
    event.corr            # Corr 分数
    event.ioe             # IOE 效率
    
    # 录像数据
    event.raw_data        # base64 编码的录像
    event.mode            # 游戏模式
```

### BoardUpdateEvent - 棋盘更新

每步操作都会触发：

```python
def _on_board_update(self, event: BoardUpdateEvent):
    event.board           # 当前棋盘状态
    event.mouse_state     # 鼠标状态
```

## 取消订阅

```python
def _some_method(self):
    self.unsubscribe(VideoSaveEvent)
```

## 事件处理器注意事项

**重要**: 事件处理器在**插件工作线程**中执行：

- ✅ 可以做 IO 操作（数据库、文件）
- ✅ 可以做耗时计算
- ❌ 不能直接操作 GUI（会导致崩溃）
- ✅ 更新 GUI 必须使用信号槽或 `run_on_gui()`

## 事件处理流程

```
主进程触发事件
    ↓
ZMQ 发送到插件管理器
    ↓
EventDispatcher 分发
    ↓
各插件的 handler 被调用（在工作线程）
    ↓
handler 处理数据
    ↓
（如需更新 GUI）通过信号槽投递到主线程
```
