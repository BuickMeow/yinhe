/// PR 顶部“其他轨道”显示状态。
///
/// `showAll == false` = 只显示当前编辑轨；`gray == true` = 其他轨道去色。
/// 彩色/黑白两个菜单项最多选中一个，重复点击已选项会关闭。
typedef PrTrackDisplay = ({bool showAll, bool gray});

/// 点击“显示其他轨道（彩色/黑白）”后的下一状态。
///
/// [pickGray] = 点的是黑白项还是彩色项。已选中的项再点一次会关闭，
/// 回到只显示当前轨；点另一项则直接切换。
PrTrackDisplay togglePrTrackDisplay({
  required bool showAll,
  required bool gray,
  required bool pickGray,
}) {
  final already = showAll && gray == pickGray;
  if (already) {
    return (showAll: false, gray: false);
  }
  return (showAll: true, gray: pickGray);
}
