import React from "react";

export const ActivityPanel: React.FC = () => {
  // Placeholder activity list -- later we can wire real events/logs
  const activities = [
    {
      id: 1,
      time: "20 mins ago",
      text: "James has deleted App Brief from Payment / LinkedIn Project",
    },
    { id: 2, time: "45 mins ago", text: "Elizabeth commented on Landingpage Final" },
    { id: 3, time: "1 hour ago", text: "Jordan liked Style Guideline Final" },
  ];

  return (
    <aside className="activity-panel h-full p-4">
      <div className="font-bold mb-4">Last Activity</div>
      <div className="space-y-3 overflow-auto max-h-[calc(100vh-160px)]">
        {activities.map((a) => (
          <div key={a.id} className="text-sm text-muted-foreground">
            <div className="flex items-center gap-2">
              <div className="w-8 h-8 rounded-full bg-muted flex items-center justify-center">🤖</div>
              <div className="flex-1">
                <div className="text-xs text-muted-foreground">{a.time}</div>
                <div className="text-sm">{a.text}</div>
              </div>
            </div>
          </div>
        ))}
      </div>
    </aside>
  );
};
