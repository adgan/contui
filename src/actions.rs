use gpui::actions;

actions!(
    dockyard,
    [
        ShowContainers,
        ShowImages,
        ShowVolumes,
        Refresh,
        FocusSearch,
        Back,
        Quit,
        RunContainer,
        PullImage
    ]
);
