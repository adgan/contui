use gpui::actions;

actions!(
    contui,
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
